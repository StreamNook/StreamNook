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
      {/* Toolbar — also acts as the dock drop zone when dragging a visible stream */}
      <div
        ref={setDockRef}
        className={`
          relative flex items-center justify-between px-4 py-2 backdrop-blur-md border-b shadow-sm
          transition-all duration-300
          ${isDragging && isOver
            ? 'bg-accent/15 border-accent/50 shadow-[0_0_25px_rgba(var(--color-accent-rgb),0.2),0_2px_10px_rgba(var(--color-accent-rgb),0.15)]'
            : isDragging
              ? 'bg-surface/50 border-accent/30 shadow-[0_0_12px_rgba(var(--color-accent-rgb),0.08)]'
              : 'bg-surface/50 border-borderSubtle'
          }
        `}
      >
        {/* Animated shimmer overlay when dragging */}
        {isDragging && (
          <div
            className="absolute inset-0 pointer-events-none overflow-hidden rounded-[inherit]"
            style={{ opacity: isOver ? 0.7 : 0.35 }}
          >
            <div
              className="absolute inset-0"
              style={{
                background: 'linear-gradient(90deg, transparent 0%, rgba(167,139,250,0.35) 40%, rgba(167,139,250,0.5) 50%, rgba(167,139,250,0.35) 60%, transparent 100%)',
                animation: 'dock-shimmer 1.5s ease-in-out infinite',
              }}
            />
          </div>
        )}

        {/* Pulsing border glow when dragging (not hovering) */}
        {isDragging && !isOver && (
          <div
            className="absolute inset-0 pointer-events-none rounded-[inherit]"
            style={{
              boxShadow: '0 0 0 1px rgba(var(--color-accent-rgb), 0.2)',
              animation: 'dock-pulse 1.5s ease-in-out infinite',
            }}
          />
        )}

        {/* Floating "Drop to dock" label */}
        <div
          className={`
            absolute left-1/2 -translate-x-1/2 bottom-0 translate-y-full
            px-3 py-1 rounded-b-md text-[10px] font-bold uppercase tracking-widest
            transition-all duration-300 pointer-events-none z-20
            ${isDragging && isOver
              ? 'opacity-100 bg-accent/25 text-accent border border-t-0 border-accent/30 backdrop-blur-md shadow-lg'
              : isDragging
                ? 'opacity-70 bg-glass/60 text-textMuted border border-t-0 border-borderSubtle backdrop-blur-md'
                : 'opacity-0'
            }
          `}
        >
          {isOver ? '↓ Release to dock' : '↑ Drag here to dock'}
        </div>

        <style>{`
          @keyframes dock-shimmer {
            0%, 100% { transform: translateX(-100%); }
            50% { transform: translateX(100%); }
          }
          @keyframes dock-pulse {
            0%, 100% { box-shadow: 0 0 0 1px rgba(167,139,250,0.15); }
            50% { box-shadow: 0 0 0 2px rgba(167,139,250,0.35), 0 0 15px rgba(167,139,250,0.1); }
          }
        `}</style>

        {/* Left side: Stats & Title */}
        <div className="flex items-center gap-4 flex-1 overflow-hidden mr-4">
          <div className="flex items-center gap-1.5 shrink-0">
            {slots.length > 0 ? (
              <>
                <Tooltip content="Browse Streams (Keep Playing)" delay={200} side="bottom">
                  <button
                    onClick={() => useAppStore.getState().toggleHome()}
                    className="flex items-center gap-1.5 px-3 py-1.5 text-sm font-bold tracking-wide rounded-lg transition-all whitespace-nowrap glass-button text-textSecondary hover:text-white"
                  >
                    <ArrowLeft size={16} />
                    <span>Browse</span>
                  </button>
                </Tooltip>

                <Tooltip content="Exit (Kill Streams)" delay={200} side="bottom">
                  <button
                    onClick={toggleMultiNook}
                    className="flex items-center justify-center w-[32px] h-[32px] rounded-lg transition-all glass-button text-textSecondary hover:text-red-400 hover:bg-red-500/10 shrink-0"
                  >
                    <X size={16} />
                  </button>
                </Tooltip>
              </>
            ) : (
              <Tooltip content="Close Grid Engine" delay={200} side="bottom">
                <button
                  onClick={toggleMultiNook}
                  className="flex items-center gap-1.5 px-3 py-1.5 text-sm font-bold tracking-wide rounded-lg transition-all whitespace-nowrap glass-button text-textSecondary hover:text-white hover:text-red-400 hover:bg-red-500/10 group"
                >
                  <ArrowLeft size={16} className="group-hover:-translate-x-0.5 transition-transform" />
                  <span>Exit</span>
                </button>
              </Tooltip>
            )}
          </div>

          {(minimizedSlots.length > 0 || (slots.length === 0 && isTutorialDocked)) && (
            <>
              <div className="h-4 w-px bg-borderSubtle shrink-0" />
              <div className="flex items-center gap-2 flex-1 overflow-x-auto scrollbar-none mask-edges">
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
            </>
          )}
        </div>

        {/* Right side: Add Stream & Controls */}
        <div className="flex items-center gap-3 relative z-30">
          {/* Add Stream — Collapsible search */}
          <div ref={searchContainerRef} className="relative">
            <div className={`
              flex items-center rounded-full transition-all duration-300 overflow-hidden
              ${isSearchOpen
                ? 'w-56 glass-input'
                : 'w-8 h-8 glass-button group cursor-pointer text-textSecondary hover:text-white'
              }
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
                <Tooltip content="Add Stream" delay={200} side="bottom">
                  <button
                    onClick={() => {
                      if (slots.length < 25) setIsSearchOpen(true);
                    }}
                    disabled={slots.length >= 25}
                    className="w-full h-full flex items-center justify-center transition-colors disabled:opacity-40"
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

          <div className="flex items-center gap-1.5 ml-2 border-l border-white/10 pl-2">
            {/* Presets. Saved, named channel sets openable in one click */}
            <MultiNookPresets />

            {/* Resync Toggle */}
            <Tooltip content="Resynchronize Playback (Force Reload All)" delay={200} side="bottom">
              <button
                onClick={resyncAllSlots}
                disabled={slots.length === 0}
                className={`w-8 h-8 flex items-center justify-center transition-all duration-200 glass-button text-textSecondary hover:text-accent active:scale-95 disabled:opacity-50 disabled:cursor-not-allowed`}
              >
                <RefreshCcw size={15} />
              </button>
            </Tooltip>

            {/* Mute All Toggle — cuts audio on every tile at once. Per-tile mute
                state is untouched, so unmuting restores the previous audio focus. */}
            <Tooltip content={isAllMuted ? 'Unmute All' : 'Mute All'} delay={200} side="bottom">
              <button
                onClick={toggleAllMuted}
                disabled={slots.length === 0}
                aria-pressed={isAllMuted}
                className={`w-8 h-8 flex items-center justify-center transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed ${
                  isAllMuted
                    ? 'glass-button-active text-error drop-shadow-md'
                    : 'glass-button text-textSecondary hover:text-error'
                }`}
              >
                {isAllMuted ? <VolumeX size={15} /> : <Volume2 size={15} />}
              </button>
            </Tooltip>

            {/* Mod View Toggle — shows/hides the Moderator Logs pane (persisted) */}
            <Tooltip content={showModLogs ? 'Hide Mod View' : 'Show Mod View'} delay={200} side="bottom">
              <button
                onClick={toggleModLogs}
                aria-pressed={showModLogs}
                className={`w-8 h-8 flex items-center justify-center transition-all duration-200 ${
                  showModLogs
                    ? 'glass-button-active text-success drop-shadow-md'
                    : 'glass-button text-error/80 hover:text-success'
                }`}
              >
                <ShieldCheck size={15} />
              </button>
            </Tooltip>

            {/* Chat Toggle */}
          <Tooltip content={isChatHidden ? 'Show Chat' : 'Hide Chat'} delay={200} side="bottom">
            <button
              onClick={toggleChatHidden}
              className={`w-8 h-8 flex items-center justify-center transition-all duration-200 ${
                isChatHidden
                  ? 'glass-button-active text-accent drop-shadow-md'
                  : 'glass-button text-textSecondary hover:text-error hover:bg-error/10'
              }`}
            >
              {isChatHidden ? <MessageSquareOff size={15} /> : <MessageSquare size={15} />}
            </button>
          </Tooltip>
          </div>

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
        className={`
          group flex items-center gap-2 pl-1 pr-1 py-1 rounded-full glass-button
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
        className={`
          group flex items-center gap-2 pl-1 pr-1 py-1 rounded-full glass-button
          cursor-grab active:cursor-grabbing border-emerald-400/30 bg-emerald-400/10
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

