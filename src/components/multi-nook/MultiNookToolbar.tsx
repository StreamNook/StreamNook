import React, { useState, useEffect, useRef, useCallback } from 'react';
import { useDroppable, useDraggable } from '@dnd-kit/core';
import { CSS } from '@dnd-kit/utilities';
import { usemultiNookStore } from '../../stores/multiNookStore';
import { useTutorialStore } from '../../stores/tutorialStore';
import { MultiNookSlot } from '../../types';
import { makeKey } from '../../utils/providerKey';
import { GRID_PICKER_PROVIDERS, gridRefusal } from '../../types/providers';
import { Plus, Maximize2, Minimize2, MessageSquare, MessageSquareOff, Loader2, X, ArrowLeft, RefreshCcw, ShieldCheck, Search, Radio, Volume2, VolumeX } from 'lucide-react';
import { Tooltip } from '../ui/Tooltip';
import { useAppStore } from '../../stores/AppStore';
import { ChannelItem, useChannelSearch, itemKey, parseTypedChannel } from './channelSearch';
import { ChannelResultRow } from './ChannelResultRow';
import MultiNookPresets from './MultiNookPresets';

interface MultiNookToolbarProps {
  isDragging?: boolean;
  dockDropId?: string;
  dockedPrefix?: string;
}

const MultiNookToolbar: React.FC<MultiNookToolbarProps> = ({
  isDragging = false,
  dockDropId = 'dock-drop-zone',
  dockedPrefix = 'docked::',
}) => {
  const { slots, addSlot, undockSlot, swapDockedSlot, isChatHidden, toggleChatHidden, toggleMultiNook, resyncAllSlots, isAllMuted, toggleAllMuted } = usemultiNookStore();
  const minimizedSlots = slots.filter(s => s.isMinimized);
  const { isDocked: isTutorialDocked, setIsDocked: setTutorialDocked } = useTutorialStore();

  // Mod-view (Moderator Logs pane) visibility — the global, now-persisted setting.
  const showModLogs = useAppStore((s) => s.settings.show_mod_logs ?? false);
  const toggleModLogs = useCallback(() => {
    const st = useAppStore.getState();
    st.updateSettings({ ...st.settings, show_mod_logs: !(st.settings.show_mod_logs ?? false) });
  }, []);

  const { setNodeRef: setDockRef, isOver } = useDroppable({ id: dockDropId });

  // --- Add Channel Search (collapsible panel) ---
  const [isSearchOpen, setIsSearchOpen] = useState(false);
  const [isAdding, setIsAdding] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const searchContainerRef = useRef<HTMLDivElement>(null);
  // Channels already in the grid, excluded from every list so you can't add a
  // duplicate. Keyed COMPOSITE (provider:login), matching the store's slotKey:
  // a bare login would make one Twitch tile hide the same-named Kick channel,
  // which is exactly the pair the composite key exists to keep apart.
  //
  // Deliberately NOT wrapped in useMemo: react-hooks/preserve-manual-memoization
  // rejects memoizing on `slots` here and is an error in this repo, so a fresh
  // Set per render is the shape the lint allows.
  const existingKeys = new Set(slots.map((s) => makeKey(s.provider ?? 'twitch', s.channelLogin)));

  // Shared finder: live following + debounced multi-platform search + keyboard nav.
  const {
    searchInput,
    setSearchInput,
    query,
    isSearching,
    followingItems,
    searchItems,
    visibleItems,
    followedCount,
    highlightIndex,
    setHighlightIndex,
    listRef,
    refreshFollowing,
    reset: resetSearch,
  } = useChannelSearch({ excludeKeys: existingKeys, providers: GRID_PICKER_PROVIDERS });

  // Focus input when the panel opens, and refresh the live-following list so it's
  // current the moment the panel appears.
  useEffect(() => {
    if (isSearchOpen) {
      refreshFollowing();
      // Small delay for the expand animation to start
      const t = setTimeout(() => inputRef.current?.focus({ preventScroll: true }), 80);
      return () => clearTimeout(t);
    }
  }, [isSearchOpen, refreshFollowing]);

  const closeSearch = () => {
    setIsSearchOpen(false);
    resetSearch();
  };

  // Click outside to close. Inlines the close behaviour (rather than depending on
  // the unmemoized closeSearch) so the listener only re-binds when the panel
  // opens/closes; resetSearch is stable from the hook.
  useEffect(() => {
    if (!isSearchOpen) return;
    function handleClickOutside(e: MouseEvent) {
      if (searchContainerRef.current && !searchContainerRef.current.contains(e.target as Node)) {
        setIsSearchOpen(false);
        resetSearch();
      }
    }
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, [isSearchOpen, resetSearch]);

  const handleSelectItem = async (item: ChannelItem) => {
    if (!item.login) return;
    setIsAdding(true);
    await addSlot(item.login, item.provider ?? 'twitch');
    closeSearch();
    setIsAdding(false);
  };

  const handleKeyDown = async (e: React.KeyboardEvent) => {
    if (e.key === 'Escape') {
      closeSearch();
      return;
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setHighlightIndex(i => Math.min(i + 1, Math.max(visibleItems.length - 1, 0)));
      return;
    }
    if (e.key === 'ArrowUp') {
      e.preventDefault();
      setHighlightIndex(i => Math.max(i - 1, 0));
      return;
    }
    if (e.key === 'Enter') {
      const target = visibleItems[highlightIndex];
      if (target) {
        e.preventDefault();
        await handleSelectItem(target);
      } else if (searchInput.trim() && !isSearching) {
        // Fallback for a channel the search did not surface. Typed text carries no
        // platform, so it is read through parseTypedChannel: a bare name is
        // Twitch, and the app's own `kick:xqc` form adds a Kick channel instead
        // of a Twitch channel literally named "kick:xqc".
        const typed = parseTypedChannel(searchInput);
        if (!typed) {
          useAppStore.getState().addToast(`"${searchInput.trim()}" is not a channel name`, 'info');
          return;
        }
        e.preventDefault();
        setIsAdding(true);
        await addSlot(typed.channel, typed.provider);
        closeSearch();
        setIsAdding(false);
      }
    }
  };

  return (
    <div className="relative z-10" style={{ WebkitAppRegion: 'no-drag' } as React.CSSProperties}>
      {/* The strip that carries MultiNook's own controls.

          It paints NOTHING. It used to be a full-bleed `bg-surface/50` row with a
          hard bottom border, which put a second horizontal bar in the old visual
          language directly beneath the floating, glazed title bar: a
          double-decker, with the lower deck flat and bordered. What reads as a
          bar here is now only the two glaze clusters, in exactly the material
          the title bar 40px above wears, floating over the app background.

          It still RESERVES its height, and that is deliberate rather than
          leftover. A tile's own chrome is anchored to the top of the tile
          (identity on the left, follow/spotlight/dock/close on the right), so
          clusters hovering over the first row would sit on top of the very
          controls the pointer went there for. Reserving the strip costs 44px of
          grid and buys no collision anywhere; see StreamNook_Floating_Title_Bar
          for the same decision taken the other way, one level up, where nothing
          was underneath to collide with.

          Also still the dock drop zone: the whole strip accepts a dragged tile,
          which is where the gesture has always been aimed. */}
      <div
        ref={setDockRef}
        className="relative flex h-11 items-center justify-between bg-transparent px-3"
      >
        {/* The drop target, drawn only while a tile is in the air.

            It used to be a full-bleed shimmer sweeping the whole bar, in a
            hardcoded violet that ignored the theme. With the bar itself gone
            there is no surface for a sweep to travel across, so the target is
            now a bounded outline sitting inside the strip's own gutter: a
            dashed rule while the tile is merely airborne, a solid accent ring
            once it is over the strip. Both derive from --color-accent, so a
            theme change carries. */}
        {isDragging && (
          <div
            aria-hidden
            className={`
              pointer-events-none absolute inset-x-2 inset-y-1 rounded-full
              flex items-center justify-center
              transition-[background-color,border-color,box-shadow] duration-200
              ${isOver
                ? 'border border-accent/60 bg-accent/10 shadow-[0_0_20px_rgba(var(--color-accent-rgb),0.12)_inset]'
                : 'border border-dashed border-white/15'
              }
            `}
          >
            {/* The label only when there is room for it. Docked pills occupy
                the middle of the strip, which is the only place a centred
                label can go, and a caption reading through a row of channel
                names is worse than no caption: the outline alone already says
                "target". */}
            {minimizedSlots.length === 0 && (
              <span
                className={`text-[10px] font-bold uppercase tracking-widest transition-colors duration-200 ${
                  isOver ? 'text-accent' : 'text-textMuted'
                }`}
              >
                {isOver ? 'Release to dock' : 'Drag here to dock'}
              </span>
            )}
          </div>
        )}

        {/* Leaving the grid, either way round: Browse keeps every tile playing
            behind Home, Exit stops them. Two ways out of one surface, so they
            share a cluster and a hairline separates them. */}
        <div className="flex items-center gap-3 flex-1 overflow-hidden mr-4">
          <div className="chrome-glaze titlebar-icon-group shrink-0">
            {slots.length > 0 ? (
              <>
                <Tooltip content="Browse without stopping the streams" delay={200} side="bottom">
                  <button
                    onClick={() => useAppStore.getState().toggleHome()}
                    className="titlebar-icon-btn gap-1.5 !px-2.5"
                  >
                    <ArrowLeft size={16} />
                    <span className="text-[12.5px] font-semibold">Browse</span>
                  </button>
                </Tooltip>
                <span className="mx-0.5 h-4 w-px bg-borderSubtle" aria-hidden />
                <Tooltip content="Leave MultiNook and stop every stream" delay={200} side="bottom">
                  <button
                    onClick={toggleMultiNook}
                    className="titlebar-icon-btn hover:!text-error hover:!bg-error/10"
                  >
                    <X size={16} />
                  </button>
                </Tooltip>
              </>
            ) : (
              <Tooltip content="Leave MultiNook" delay={200} side="bottom">
                <button
                  onClick={toggleMultiNook}
                  className="titlebar-icon-btn gap-1.5 !px-2.5 group hover:!text-error hover:!bg-error/10"
                >
                  <ArrowLeft size={16} className="transition-transform group-hover:-translate-x-0.5" />
                  <span className="text-[12.5px] font-semibold">Exit</span>
                </button>
              </Tooltip>
            )}
          </div>

          {/* Docked streams live between the two clusters, scrolling if there
              are more of them than fit. Still mounted and still playing; the
              pill is how you bring one back. */}
          {(minimizedSlots.length > 0 || (slots.length === 0 && isTutorialDocked)) && (
            <div className="flex items-center gap-2 flex-1 overflow-x-auto scrollbar-none mask-edges py-1">
                {minimizedSlots.map((slot) => (
                  <DraggableDockPill
                    key={slot.id}
                    slot={slot}
                    dockedPrefix={dockedPrefix}
                    onSwap={() => swapDockedSlot(slot.id)}
                    onUndock={() => undockSlot(slot.id)}
                  />
                ))}
                
                {slots.length === 0 && isTutorialDocked && (
                  <TutorialDockPill onUndock={() => setTutorialDocked(false)} />
                )}
              </div>
          )}
        </div>

        {/* Everything that acts ON the grid, in one cluster wearing the title
            bar's material. Three jobs, hairline-separated in the order you reach
            for them: put a stream in (add, presets), fix the streams that are in
            (resync, mute all), decide what is shown beside them (mod logs,
            chat). The cluster is one surface so the eleven-ish controls read as
            a toolkit rather than as a row of loose buttons. */}
        <div className="chrome-glaze titlebar-icon-group relative z-30 shrink-0">
          {/* Add Stream — Collapsible search */}
          <div ref={searchContainerRef} className="relative">
            {/* Collapsed, this is one more icon in the cluster and wears no
                surface of its own: a glass button inside a glass cluster is two
                materials arguing. Open, it becomes a real input and takes the
                recessed field treatment, which is the cue that it now accepts
                typing. */}
            <div className={`
              flex items-center rounded-full transition-all duration-300 overflow-hidden
              ${isSearchOpen ? 'w-56 glass-input' : 'w-8 h-[30px]'}
            `}>
              {isSearchOpen ? (
                <>
                  <input
                    ref={inputRef}
                    type="text"
                    value={searchInput}
                    onChange={(e) => setSearchInput(e.target.value)}
                    onKeyDown={handleKeyDown}
                    placeholder="Search or pick a live channel..."
                    className="bg-transparent border-none text-sm text-textPrimary placeholder:text-textMuted flex-1 px-3 py-1.5 outline-none h-8"
                    disabled={isAdding || slots.length >= 25}
                  />
                  {isSearching ? (
                    <div className="pr-2 flex items-center">
                      <Loader2 size={14} className="text-accent animate-spin" />
                    </div>
                  ) : searchInput && (
                    <button
                      onClick={closeSearch}
                      className="pr-2 text-textMuted hover:text-textPrimary transition-colors"
                    >
                      <X size={14} />
                    </button>
                  )}
                </>
              ) : (
                <Tooltip
                  content={slots.length >= 25 ? 'The grid is full (25 streams)' : 'Add a stream'}
                  delay={200}
                  side="bottom"
                >
                  <button
                    onClick={() => {
                      if (slots.length < 25) setIsSearchOpen(true);
                    }}
                    disabled={slots.length >= 25}
                    className="titlebar-icon-btn !min-w-0 w-full h-full disabled:opacity-40"
                  >
                    <Plus size={16} />
                  </button>
                </Tooltip>
              )}
            </div>

            {/* Smart list — live following on open, instant filter + Twitch search while typing */}
            {isSearchOpen && (
              <div className="absolute right-0 top-full mt-2 w-72 z-50">
                {/* Frosted glass surface — explicit opaque base because this menu floats directly
                    over the (bright) video grid with no dimming scrim, where the glass-strength
                    tint alone reads as see-through. */}
                <div
                  className="liquid-glass-panel overflow-hidden"
                  style={{ backgroundColor: 'rgba(16, 16, 20, 0.92)' }}
                >
                  <div ref={listRef} className="max-h-80 overflow-y-auto custom-scrollbar p-1.5">

                    {/* Live following (instant, from cache) */}
                    {followingItems.length > 0 && (
                      <>
                        <div className="px-2.5 pt-1.5 pb-1 flex items-center gap-1.5">
                          <Radio size={11} className="text-red-500" />
                          <span className="text-[10px] font-bold uppercase tracking-wider text-textMuted">
                            {query ? 'Following · live' : 'Live now'}
                          </span>
                        </div>
                        <div className="space-y-0.5">
                          {followingItems.map((item, i) => (
                            <ChannelResultRow
                              key={`f-${itemKey(item)}`}
                              item={item}
                              index={i}
                              highlighted={highlightIndex === i}
                              disabled={isAdding}
                              reason={gridRefusal(item.provider ?? 'twitch')}
                              onSelect={handleSelectItem}
                              onHover={setHighlightIndex}
                            />
                          ))}
                        </div>
                      </>
                    )}

                    {/* Channel search across every platform the grid accepts (debounced) */}
                    {query && (searchItems.length > 0 || isSearching) && (
                      <>
                        <div className="px-2.5 pt-2 pb-1 flex items-center gap-1.5">
                          <Search size={11} className="text-textMuted" />
                          <span className="text-[10px] font-bold uppercase tracking-wider text-textMuted">
                            All channels
                          </span>
                          {isSearching && <Loader2 size={11} className="text-accent animate-spin ml-auto" />}
                        </div>
                        <div className="space-y-0.5">
                          {searchItems.map((item, i) => {
                            const idx = followingItems.length + i;
                            return (
                              <ChannelResultRow
                                key={`s-${itemKey(item)}`}
                                item={item}
                                index={idx}
                                highlighted={highlightIndex === idx}
                                disabled={isAdding}
                                reason={gridRefusal(item.provider ?? 'twitch')}
                                onSelect={handleSelectItem}
                                onHover={setHighlightIndex}
                              />
                            );
                          })}
                        </div>
                      </>
                    )}

                    {/* Empty states */}
                    {visibleItems.length === 0 && (
                      query ? (
                        isSearching ? (
                          <div className="px-4 py-5 flex items-center justify-center gap-2.5">
                            <Loader2 size={14} className="text-accent animate-spin" />
                            <span className="text-xs text-textSecondary font-medium">Searching...</span>
                          </div>
                        ) : (
                          <div className="px-4 py-5 text-center">
                            <span className="text-xs text-textMuted">No channels found for "{searchInput}"</span>
                          </div>
                        )
                      ) : (
                        <div className="px-4 py-5 text-center">
                          <span className="text-xs text-textMuted">
                            {followedCount === 0
                              ? 'No followed channels are live. Type to search.'
                              : 'Start typing to search any channel'}
                          </span>
                        </div>
                      )
                    )}
                  </div>
                </div>
              </div>
            )}
          </div>

          {/* Presets. Saved, named channel sets openable in one click */}
          <MultiNookPresets />

          <span className="mx-0.5 h-4 w-px bg-borderSubtle" aria-hidden />

          {/* Resync — force every tile to reload so co-streams line up again. */}
          <Tooltip content="Resync playback on every stream" delay={200} side="bottom">
            <button
              onClick={resyncAllSlots}
              disabled={slots.length === 0}
              className="titlebar-icon-btn hover:!text-accent active:scale-95 disabled:opacity-40 disabled:cursor-not-allowed"
            >
              <RefreshCcw size={16} />
            </button>
          </Tooltip>

          {/* Mute All Toggle — cuts audio on every tile at once. Per-tile mute
              state is untouched, so unmuting restores the previous audio focus. */}
          <Tooltip content={isAllMuted ? 'Unmute every stream' : 'Mute every stream'} delay={200} side="bottom">
            <button
              onClick={toggleAllMuted}
              disabled={slots.length === 0}
              aria-pressed={isAllMuted}
              className={`titlebar-icon-btn disabled:opacity-40 disabled:cursor-not-allowed ${
                isAllMuted ? 'is-active !text-error' : 'hover:!text-error'
              }`}
            >
              {isAllMuted ? <VolumeX size={16} /> : <Volume2 size={16} />}
            </button>
          </Tooltip>

          <span className="mx-0.5 h-4 w-px bg-borderSubtle" aria-hidden />

          {/* Mod View Toggle — shows/hides the Moderator Logs pane (persisted) */}
          <Tooltip content={showModLogs ? 'Hide mod logs' : 'Show mod logs'} delay={200} side="bottom">
            <button
              onClick={toggleModLogs}
              aria-pressed={showModLogs}
              className={`titlebar-icon-btn ${showModLogs ? 'is-active !text-success' : 'hover:!text-success'}`}
            >
              <ShieldCheck size={16} />
            </button>
          </Tooltip>

          {/* Chat Toggle */}
          <Tooltip content={isChatHidden ? 'Show chat' : 'Hide chat'} delay={200} side="bottom">
            <button
              onClick={toggleChatHidden}
              aria-pressed={isChatHidden}
              className={`titlebar-icon-btn ${isChatHidden ? 'is-active' : 'hover:!text-error'}`}
            >
              {isChatHidden ? <MessageSquareOff size={16} /> : <MessageSquare size={16} />}
            </button>
          </Tooltip>

        </div>
      </div>
    </div>
  );
};

/** Draggable pill for a docked/minimized stream */
const DraggableDockPill: React.FC<{
  slot: MultiNookSlot;
  dockedPrefix: string;
  onSwap: () => void;
  onUndock: () => void;
}> = ({ slot, dockedPrefix, onSwap, onUndock }) => {
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    isDragging: isPillDragging,
  } = useDraggable({
    id: `${dockedPrefix}${slot.id}`,
  });

  const style = transform
    ? {
        transform: CSS.Translate.toString(transform),
        zIndex: isPillDragging ? 50 : undefined,
      }
    : undefined;

  return (
    <Tooltip content="Drag to grid to restore · Click to swap" delay={500} side="bottom">
      <div
        ref={setNodeRef}
        style={style}
        // Glaze, not glass. The pill sits between the two glaze clusters, so a
        // flat glass button here put a third material in a strip that is only
        // 44px tall. `--control` is the variant for a glazed surface that IS
        // the button: it lifts under the pointer instead of relying on a
        // separate hover fill.
        className={`
          group flex h-[30px] items-center gap-2 pl-1 pr-1 chrome-glaze chrome-glaze--control
          cursor-grab active:cursor-grabbing
          transition-all duration-300 shrink-0 touch-none
          ${isPillDragging
            ? 'opacity-80 scale-105 shadow-[0_0_20px_rgba(var(--color-accent-rgb),0.3)] ring-1 ring-accent'
            : 'hover:text-white'
          }
        `}
        onClick={() => !isPillDragging && onSwap()}
        {...attributes}
        {...listeners}
      >
        <div className="flex items-center gap-1.5 opacity-80 group-hover:opacity-100 transition-opacity">
        {slot.profileImageUrl ? (
          <img src={slot.profileImageUrl} alt="" className="w-5 h-5 rounded-full object-cover shadow-sm bg-black/20" />
        ) : (
          <div className="w-2 h-2 ml-2 rounded-full bg-accent animate-pulse"></div>
        )}
        <span className="text-xs font-semibold text-textPrimary truncate max-w-[100px] select-none pr-1">
          {slot.channelName || slot.channelLogin}
        </span>
      </div>
      <Tooltip content="Restore to Grid" delay={200} side="bottom">
        <button
          onClick={(e) => {
            e.stopPropagation();
            onUndock();
          }}
          className="w-5 h-5 flex items-center justify-center rounded-full bg-accent/10 text-accent hover:bg-accent hover:text-white transition-all ml-1"
          onPointerDown={(e) => e.stopPropagation()}
        >
          <Maximize2 size={10} strokeWidth={3} />
        </button>
      </Tooltip>
    </div>
  </Tooltip>
  );
};

/** Draggable fake pill for the tutorial */
const TutorialDockPill: React.FC<{
  onUndock: () => void;
}> = ({ onUndock }) => {
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    isDragging,
  } = useDraggable({
    id: `docked::tutorial::dock`,
  });

  const style = transform
    ? {
        transform: CSS.Translate.toString(transform),
        zIndex: isDragging ? 50 : undefined,
      }
    : undefined;

  return (
    <Tooltip content="Drag to grid to restore" delay={500} side="bottom">
      <div
        ref={setNodeRef}
        style={style}
        // Same material as a real dock pill, tinted so the tutorial's stand-in
        // is obviously not one of your streams.
        className={`
          group flex h-[30px] items-center gap-2 pl-1 pr-1 chrome-glaze chrome-glaze--control
          cursor-grab active:cursor-grabbing ring-1 ring-emerald-400/30
          transition-all duration-300 shrink-0 touch-none
          ${isDragging
            ? 'opacity-80 scale-105 shadow-[0_0_20px_color-mix(in_srgb,var(--color-success)_30%,transparent)] ring-1 ring-emerald-400'
            : 'hover:text-white'
          }
        `}
        {...attributes}
        {...listeners}
      >
        <div className="flex items-center gap-1.5 opacity-80 group-hover:opacity-100 transition-opacity">
        <div className="w-5 h-5 rounded-full bg-emerald-400/20 flex items-center justify-center">
            <Minimize2 size={12} className="text-emerald-400" />
        </div>
        <span className="text-xs font-semibold text-emerald-400 truncate max-w-[130px] select-none pr-1">
          Docking Tutorial
        </span>
      </div>
      <Tooltip content="Restore to Grid" delay={200} side="bottom">
        <button
          onClick={(e) => {
            e.stopPropagation();
            onUndock();
          }}
          className="w-5 h-5 flex items-center justify-center rounded-full bg-emerald-400/20 text-emerald-400 hover:bg-emerald-400 hover:text-white transition-all ml-1"
          onPointerDown={(e) => e.stopPropagation()}
        >
          <Maximize2 size={10} strokeWidth={3} />
        </button>
      </Tooltip>
    </div>
  </Tooltip>
  );
};

export default MultiNookToolbar;

