import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  Bookmark,
  BookmarkPlus,
  ListPlus,
  Pencil,
  Trash2,
  Copy,
  Play,
  ArrowLeft,
  Check,
  X,
  Plus,
  Search,
  Loader2,
  Radio,
  Image as ImageIcon,
  Gamepad2,
  Users,
  Square,
} from 'lucide-react';
import { Tooltip } from '../ui/Tooltip';
import { useMultiNookPresetsStore, slotToPresetChannel } from '../../stores/multiNookPresetsStore';
import { usemultiNookStore } from '../../stores/multiNookStore';
import { MultiNookPreset, MultiNookPresetChannel, MultiNookPresetIcon } from '../../types';
import { ChannelItem, useChannelSearch, DEFAULT_AVATAR, itemKey, parseTypedChannel } from './channelSearch';
import { ChannelResultRow } from './ChannelResultRow';
import { Logger } from '../../utils/logger';
import { makeKey } from '../../utils/providerKey';
import { GRID_PICKER_PROVIDERS, gridRefusal } from '../../types/providers';

const MAX_NAME_LEN = 40;

/** Derive a sensible default preset name from the grid's majority game category. */
function suggestNameFromSlots(): string {
  const slots = usemultiNookStore.getState().slots;
  const counts: Record<string, number> = {};
  let best = '';
  let bestN = 0;
  for (const s of slots) {
    const g = s.gameName?.trim();
    if (!g) continue;
    counts[g] = (counts[g] || 0) + 1;
    if (counts[g] > bestN) {
      bestN = counts[g];
      best = g;
    }
  }
  return best || 'New Preset';
}

/** The thumbnail to show for a preset: its custom icon, else its first channel's
 *  avatar, else null (caller falls back to a bookmark glyph). */
function presetThumb(preset: MultiNookPreset): { src: string; round: boolean } | null {
  if (preset.icon) return { src: preset.icon.imageUrl, round: preset.icon.type === 'channel' };
  const firstWithAvatar = preset.channels.find((c) => c.profileImageUrl);
  if (firstWithAvatar?.profileImageUrl) return { src: firstWithAvatar.profileImageUrl, round: true };
  return null;
}

type View =
  | { mode: 'list' }
  | {
      mode: 'editor';
      editingId: string | null;
      seed: MultiNookPresetChannel[];
      seedName: string;
      seedIcon?: MultiNookPresetIcon;
    };

const MultiNookPresets: React.FC = () => {
  const { presets, hydrate, hydrated, applyPreset, stopActivePreset } = useMultiNookPresetsStore();
  const slots = usemultiNookStore((s) => s.slots);
  const activePresetId = usemultiNookStore((s) => s.activePresetId);

  // The equipped preset (if its id still resolves to a saved preset).
  const activePreset = presets.find((p) => p.id === activePresetId) ?? null;
  const activeThumb = activePreset ? presetThumb(activePreset) : null;

  const [isOpen, setIsOpen] = useState(false);
  const [view, setView] = useState<View>({ mode: 'list' });
  const containerRef = useRef<HTMLDivElement>(null);

  // Seed the in-memory list from persisted settings the first time the toolbar mounts.
  useEffect(() => {
    if (!hydrated) hydrate();
  }, [hydrated, hydrate]);

  const close = useCallback(() => {
    setIsOpen(false);
    setView({ mode: 'list' });
  }, []);

  // Click-outside closes the whole popover (the editor's inline pickers live
  // inside this container, so their clicks don't trip this).
  useEffect(() => {
    if (!isOpen) return;
    function onDown(e: MouseEvent) {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        close();
      }
    }
    document.addEventListener('mousedown', onDown);
    return () => document.removeEventListener('mousedown', onDown);
  }, [isOpen, close]);

  const openNew = useCallback(() => {
    setView({ mode: 'editor', editingId: null, seed: [], seedName: '' });
  }, []);

  const openSaveCurrent = useCallback(() => {
    // One definition of the preset-channel shape, so it cannot drift from
    // slotToPresetChannel and silently drop a field (it just did, with provider).
    const seed = usemultiNookStore.getState().slots.map(slotToPresetChannel);
    setView({ mode: 'editor', editingId: null, seed, seedName: suggestNameFromSlots() });
  }, []);

  const openEdit = useCallback((preset: MultiNookPreset) => {
    setView({
      mode: 'editor',
      editingId: preset.id,
      seed: preset.channels,
      seedName: preset.name,
      seedIcon: preset.icon,
    });
  }, []);

  const handleLoad = useCallback(
    async (id: string, mode: 'replace' | 'append') => {
      close();
      await applyPreset(id, mode);
    },
    [applyPreset, close],
  );

  const handleStop = useCallback(async () => {
    close();
    await stopActivePreset();
  }, [stopActivePreset, close]);

  return (
    <div ref={containerRef} className="relative">
      <Tooltip content={activePreset ? `Preset: ${activePreset.name}` : 'Presets'} delay={200} side="bottom">
        <button
          onClick={() => (isOpen ? close() : setIsOpen(true))}
          aria-pressed={isOpen}
          className={`w-8 h-8 flex items-center justify-center overflow-hidden transition-all duration-200 ${
            activeThumb
              ? `ring-1 ${isOpen ? 'ring-white/30' : 'ring-white/15 hover:ring-white/30'}`
              : isOpen
                ? 'glass-button-active text-accent'
                : 'glass-button text-textSecondary hover:text-accent'
          }`}
          style={{ borderRadius: '8px' }}
        >
          {activeThumb ? (
            // When a preset is equipped its icon IS the button (fills it), so it's
            // clearly visible at a glance instead of a tiny thumbnail lost inside
            // the glass-button chrome.
            <img
              src={activeThumb.src}
              alt=""
              onError={(e) => {
                const img = e.currentTarget as HTMLImageElement;
                if (img.src !== DEFAULT_AVATAR) img.src = DEFAULT_AVATAR;
              }}
              className="w-full h-full object-cover"
            />
          ) : (
            <Bookmark size={15} className={activePreset ? 'text-accent' : undefined} />
          )}
        </button>
      </Tooltip>

      {isOpen && (
        <div
          className={`absolute right-0 top-full mt-2 z-50 transition-[width] duration-200 ${
            view.mode === 'editor' ? 'w-[30rem]' : 'w-80'
          }`}
        >
          <div
            className="liquid-glass-panel overflow-hidden"
            style={{ backgroundColor: 'rgba(16, 16, 20, 0.92)' }}
          >
            {view.mode === 'list' ? (
              <PresetListView
                presets={presets}
                hasGrid={slots.length > 0}
                activePreset={activePreset}
                onNew={openNew}
                onSaveCurrent={openSaveCurrent}
                onEdit={openEdit}
                onLoad={handleLoad}
                onStop={handleStop}
              />
            ) : (
              <PresetEditorView
                editingId={view.editingId}
                seed={view.seed}
                seedName={view.seedName}
                seedIcon={view.seedIcon}
                onDone={() => setView({ mode: 'list' })}
              />
            )}
          </div>
        </div>
      )}
    </div>
  );
};

/* ------------------------------------------------------------------ */
/* List view                                                           */
/* ------------------------------------------------------------------ */

const PresetListView: React.FC<{
  presets: MultiNookPreset[];
  hasGrid: boolean;
  activePreset: MultiNookPreset | null;
  onNew: () => void;
  onSaveCurrent: () => void;
  onEdit: (preset: MultiNookPreset) => void;
  onLoad: (id: string, mode: 'replace' | 'append') => void;
  onStop: () => void;
}> = ({ presets, hasGrid, activePreset, onNew, onSaveCurrent, onEdit, onLoad, onStop }) => {
  const activeThumb = activePreset ? presetThumb(activePreset) : null;
  return (
    <div className="flex flex-col">
      {/* Header */}
      <div className="flex items-center justify-between px-3 pt-2.5 pb-2">
        <div className="flex items-center gap-1.5">
          <Bookmark size={12} className="text-accent" />
          <span className="text-[10px] font-bold uppercase tracking-wider text-textMuted">
            Presets{presets.length > 0 ? ` · ${presets.length}` : ''}
          </span>
        </div>
        <Tooltip content="New preset" delay={200} side="bottom">
          <button
            onClick={onNew}
            className="w-6 h-6 flex items-center justify-center rounded-md glass-button text-textSecondary hover:text-accent"
          >
            <Plus size={13} />
          </button>
        </Tooltip>
      </div>

      {/* Now-playing banner: the equipped preset, with a Stop (eject) control */}
      {activePreset && (
        <div className="px-2 pb-2">
          <div className="flex items-center gap-2.5 px-2.5 py-2 rounded-lg bg-accent/[0.08] border border-accent/15">
            <div className="w-7 h-7 shrink-0 overflow-hidden bg-black/30 flex items-center justify-center rounded-md">
              {activeThumb ? (
                <img
                  src={activeThumb.src}
                  alt=""
                  className={`w-full h-full object-cover ${activeThumb.round ? 'rounded-full' : ''}`}
                />
              ) : (
                <Bookmark size={13} className="text-accent" />
              )}
            </div>
            <div className="flex-1 min-w-0">
              <p className="text-[10px] font-bold uppercase tracking-wider text-accent leading-none">Now playing</p>
              <p className="text-[13px] font-semibold text-textPrimary truncate mt-1 leading-none">
                {activePreset.name}
              </p>
            </div>
            <Tooltip content="Stop preset (close all its streams, keep it saved)" delay={250} side="bottom">
              <button
                onClick={onStop}
                className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg glass-button text-textSecondary hover:text-red-400 text-xs font-semibold shrink-0"
              >
                <Square size={11} className="fill-current" /> Stop
              </button>
            </Tooltip>
          </div>
        </div>
      )}

      {/* List */}
      <div className="max-h-72 overflow-y-auto custom-scrollbar px-1.5">
        {presets.length === 0 ? (
          <div className="px-3 py-6 text-center">
            <p className="text-xs text-textSecondary font-medium">No presets yet</p>
            <p className="text-[11px] text-textMuted mt-1 leading-snug">
              Save a set of streamers and open them all with one click.
            </p>
          </div>
        ) : (
          <div className="space-y-0.5 pb-1">
            {presets.map((preset) => (
              <PresetRow
                key={preset.id}
                preset={preset}
                isActive={preset.id === activePreset?.id}
                onEdit={onEdit}
                onLoad={onLoad}
              />
            ))}
          </div>
        )}
      </div>

      {/* Footer action: capture the current grid */}
      <div className="px-2 pt-1.5 pb-2 border-t border-white/[0.06]">
        <Tooltip
          content={hasGrid ? 'Save the current grid as a preset' : 'Add streams first'}
          delay={300}
          side="bottom"
        >
          <button
            onClick={onSaveCurrent}
            disabled={!hasGrid}
            className="w-full flex items-center justify-center gap-2 px-3 py-2 rounded-lg glass-button text-textSecondary hover:text-accent transition-colors disabled:opacity-40 disabled:cursor-not-allowed text-[13px] font-semibold"
          >
            <BookmarkPlus size={14} />
            Save current grid
          </button>
        </Tooltip>
      </div>
    </div>
  );
};

/** A single preset row: avatar stack + name, with load / append / edit / delete. */
const PresetRow: React.FC<{
  preset: MultiNookPreset;
  isActive: boolean;
  onEdit: (preset: MultiNookPreset) => void;
  onLoad: (id: string, mode: 'replace' | 'append') => void;
}> = ({ preset, isActive, onEdit, onLoad }) => {
  const { deletePreset, duplicatePreset } = useMultiNookPresetsStore();
  const [confirmingDelete, setConfirmingDelete] = useState(false);

  const stack = preset.channels.slice(0, 4);
  const extra = preset.channels.length - stack.length;

  return (
    <div
      className={`group flex items-center gap-2 rounded-lg pl-1.5 pr-1 py-1.5 transition-colors ${
        isActive ? 'bg-accent/[0.08]' : 'hover:bg-white/[0.05]'
      }`}
    >
      {/* Load (replace). The primary action covers the avatar+name area */}
      <button
        onClick={() => onLoad(preset.id, 'replace')}
        className="flex items-center gap-2.5 flex-1 min-w-0 text-left"
      >
        {/* Icon: a custom thumbnail when set, otherwise the overlapping avatar stack */}
        {preset.icon ? (
          <img
            src={preset.icon.imageUrl}
            alt={preset.icon.label || ''}
            onError={(e) => {
              const img = e.currentTarget as HTMLImageElement;
              if (img.src !== DEFAULT_AVATAR) img.src = DEFAULT_AVATAR;
            }}
            className={`w-7 h-7 object-cover bg-black/30 shrink-0 ring-2 ring-[#101014] ${
              preset.icon.type === 'channel' ? 'rounded-full' : 'rounded-md'
            }`}
          />
        ) : (
          <div className="flex items-center shrink-0">
            {stack.length === 0 ? (
              <div className="w-7 h-7 rounded-full bg-white/[0.05] flex items-center justify-center">
                <Bookmark size={12} className="text-textMuted" />
              </div>
            ) : (
              stack.map((ch, i) => (
                <img
                  key={ch.channelLogin + i}
                  src={ch.profileImageUrl || DEFAULT_AVATAR}
                  alt=""
                  onError={(e) => {
                    const img = e.currentTarget as HTMLImageElement;
                    if (img.src !== DEFAULT_AVATAR) img.src = DEFAULT_AVATAR;
                  }}
                  className="w-7 h-7 rounded-full object-cover ring-2 ring-[#101014] bg-black/30"
                  style={{ marginLeft: i === 0 ? 0 : -10, zIndex: stack.length - i }}
                />
              ))
            )}
            {extra > 0 && (
              <div
                className="w-7 h-7 rounded-full bg-surface ring-2 ring-[#101014] flex items-center justify-center text-[10px] font-bold text-textSecondary"
                style={{ marginLeft: -10 }}
              >
                +{extra}
              </div>
            )}
          </div>
        )}

        {/* Name + count */}
        <div className="flex-1 min-w-0">
          <span
            className={`block text-[13px] font-semibold truncate leading-tight transition-colors ${
              isActive ? 'text-accent' : 'text-textPrimary group-hover:text-accent'
            }`}
          >
            {preset.name}
          </span>
          <span
            className={`block text-[11px] truncate mt-0.5 leading-tight ${
              isActive ? 'text-accent/70' : 'text-textMuted'
            }`}
          >
            {isActive
              ? 'Playing now'
              : `${preset.channels.length} ${preset.channels.length === 1 ? 'stream' : 'streams'}`}
          </span>
        </div>
      </button>

      {/* Trailing actions */}
      {confirmingDelete ? (
        <div className="flex items-center gap-1 shrink-0 pr-0.5">
          <span className="text-[11px] text-textMuted mr-0.5">Delete?</span>
          <Tooltip content="Confirm delete" delay={150} side="bottom">
            <button
              onClick={() => deletePreset(preset.id)}
              className="w-6 h-6 flex items-center justify-center rounded-md text-red-400 hover:bg-red-500/15 transition-colors"
            >
              <Check size={13} />
            </button>
          </Tooltip>
          <Tooltip content="Cancel" delay={150} side="bottom">
            <button
              onClick={() => setConfirmingDelete(false)}
              className="w-6 h-6 flex items-center justify-center rounded-md text-textMuted hover:text-textPrimary transition-colors"
            >
              <X size={13} />
            </button>
          </Tooltip>
        </div>
      ) : (
        <div className="flex items-center gap-0.5 shrink-0 opacity-0 group-hover:opacity-100 focus-within:opacity-100 transition-opacity">
          <Tooltip content="Load (replace grid)" delay={200} side="bottom">
            <button
              onClick={() => onLoad(preset.id, 'replace')}
              className="w-6 h-6 flex items-center justify-center rounded-md text-textSecondary hover:text-accent hover:bg-accent/10 transition-colors"
            >
              <Play size={12} />
            </button>
          </Tooltip>
          <Tooltip content="Add to current grid" delay={200} side="bottom">
            <button
              onClick={() => onLoad(preset.id, 'append')}
              className="w-6 h-6 flex items-center justify-center rounded-md text-textSecondary hover:text-accent hover:bg-accent/10 transition-colors"
            >
              <ListPlus size={13} />
            </button>
          </Tooltip>
          <Tooltip content="Edit" delay={200} side="bottom">
            <button
              onClick={() => onEdit(preset)}
              className="w-6 h-6 flex items-center justify-center rounded-md text-textSecondary hover:text-accent hover:bg-accent/10 transition-colors"
            >
              <Pencil size={12} />
            </button>
          </Tooltip>
          <Tooltip content="Duplicate" delay={200} side="bottom">
            <button
              onClick={() => duplicatePreset(preset.id)}
              className="w-6 h-6 flex items-center justify-center rounded-md text-textSecondary hover:text-accent hover:bg-accent/10 transition-colors"
            >
              <Copy size={12} />
            </button>
          </Tooltip>
          <Tooltip content="Delete" delay={200} side="bottom">
            <button
              onClick={() => setConfirmingDelete(true)}
              className="w-6 h-6 flex items-center justify-center rounded-md text-textSecondary hover:text-red-400 hover:bg-red-500/10 transition-colors"
            >
              <Trash2 size={12} />
            </button>
          </Tooltip>
        </div>
      )}
    </div>
  );
};

/* ------------------------------------------------------------------ */
/* Editor view                                                         */
/* ------------------------------------------------------------------ */

type EditorSubView = 'fields' | 'icon';

const PresetEditorView: React.FC<{
  editingId: string | null;
  seed: MultiNookPresetChannel[];
  seedName: string;
  seedIcon?: MultiNookPresetIcon;
  onDone: () => void;
}> = ({ editingId, seed, seedName, seedIcon, onDone }) => {
  const { createPreset, updatePreset } = useMultiNookPresetsStore();

  const [name, setName] = useState(seedName);
  const [channels, setChannels] = useState<MultiNookPresetChannel[]>(seed);
  const [icon, setIcon] = useState<MultiNookPresetIcon | undefined>(seedIcon);
  const [saving, setSaving] = useState(false);
  const [subView, setSubView] = useState<EditorSubView>('fields');
  const [searchFocused, setSearchFocused] = useState(false);

  const nameRef = useRef<HTMLInputElement>(null);
  const channelInputRef = useRef<HTMLInputElement>(null);
  const blurTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Channels already in the preset are excluded from the finder so you can't add
  // a duplicate. Keyed COMPOSITE (provider:login), matching multiNookPresetsStore's
  // own dedupe: a bare login would refuse a Kick channel because a same-named
  // Twitch one is already in the list, and the click would just do nothing.
  const exclude = useMemo(
    () => new Set(channels.map((c) => makeKey(c.provider ?? 'twitch', c.channelLogin))),
    [channels],
  );

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
    reset,
  } = useChannelSearch({ excludeKeys: exclude, providers: GRID_PICKER_PROVIDERS });

  useEffect(() => {
    const t = setTimeout(() => nameRef.current?.focus({ preventScroll: true }), 60);
    return () => clearTimeout(t);
  }, []);

  // Identity is provider-composite here for the same reason it is in the store:
  // the same login on two platforms is two different channels.
  const addChannel = (ch: MultiNookPresetChannel) => {
    const key = makeKey(ch.provider ?? 'twitch', ch.channelLogin);
    setChannels((prev) =>
      prev.some((c) => makeKey(c.provider ?? 'twitch', c.channelLogin) === key) ? prev : [...prev, ch],
    );
  };

  const removeChannel = (ch: MultiNookPresetChannel) => {
    const key = makeKey(ch.provider ?? 'twitch', ch.channelLogin);
    setChannels((prev) => prev.filter((c) => makeKey(c.provider ?? 'twitch', c.channelLogin) !== key));
  };

  const handleSelectChannel = (item: ChannelItem) => {
    addChannel({
      // undefined for Twitch, never an explicit 'twitch'. Absent means Twitch by
      // convention, and writing it out would change the byte identity of every
      // preset row already on disk.
      provider: item.provider,
      channelLogin: item.login,
      // A search row with no platform id falls back to its login (channelSearch
      // resultToItem), so that sentinel means "no real id", not "id equals name".
      channelId: item.source === 'search' && item.id === item.login ? undefined : item.id,
      channelName: item.displayName,
      profileImageUrl: item.avatarUrl,
    });
    // Clear the query so the next channel is quick to add, and keep focus.
    setSearchInput('');
    channelInputRef.current?.focus({ preventScroll: true });
  };

  const handleSearchKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setHighlightIndex((i) => Math.min(i + 1, Math.max(visibleItems.length - 1, 0)));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setHighlightIndex((i) => Math.max(i - 1, 0));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      const target = visibleItems[highlightIndex];
      if (target) {
        handleSelectChannel(target);
      } else if (searchInput.trim() && !isSearching) {
        // Fallback: stash the raw channel (no metadata) when search surfaced
        // nothing. Read through parseTypedChannel so the app's own `kick:xqc`
        // form is understood; without it that text persists to disk forever as a
        // Twitch channel literally named "kick:xqc".
        const typed = parseTypedChannel(searchInput);
        if (!typed) return;
        addChannel({
          provider: typed.provider === 'twitch' ? undefined : typed.provider,
          channelLogin: typed.channel,
          channelName: typed.channel,
        });
        setSearchInput('');
      }
    }
  };

  const canSave = name.trim().length > 0 && channels.length > 0 && !saving;

  const handleSave = async () => {
    if (!canSave) return;
    setSaving(true);
    if (editingId) {
      // null clears any removed icon; an object sets it.
      await updatePreset(editingId, { name, channels, icon: icon ?? null });
    } else {
      await createPreset(name, channels, icon);
    }
    setSaving(false);
    onDone();
  };

  if (subView === 'icon') {
    return (
      <PresetIconPicker
        channels={channels}
        current={icon}
        onPick={(next) => {
          setIcon(next ?? undefined);
          setSubView('fields');
        }}
        onBack={() => setSubView('fields')}
      />
    );
  }

  // Results only take over the scroll body while the user is actively searching.
  const showResults = searchFocused || query.length > 0;

  return (
    <div className="flex flex-col max-h-[80vh]">
      {/* Header */}
      <div className="flex items-center gap-2 px-2.5 pt-2.5 pb-2 shrink-0">
        <button
          onClick={onDone}
          className="w-6 h-6 flex items-center justify-center rounded-md glass-button text-textSecondary hover:text-white shrink-0"
        >
          <ArrowLeft size={13} />
        </button>
        <span className="text-[10px] font-bold uppercase tracking-wider text-textMuted">
          {editingId ? 'Edit preset' : 'New preset'}
        </span>
      </div>

      {/* Fixed inputs: icon + name, then the channel finder */}
      <div className="px-2.5 pb-2 space-y-2 shrink-0">
        <div className="flex items-center gap-2">
          <Tooltip content="Set preset icon" delay={300} side="bottom">
            <button
              onClick={() => setSubView('icon')}
              className="w-9 h-9 shrink-0 rounded-lg glass-button flex items-center justify-center overflow-hidden text-textSecondary hover:text-accent"
            >
              {icon ? (
                <img
                  src={icon.imageUrl}
                  alt=""
                  className={`w-full h-full object-cover ${icon.type === 'channel' ? 'rounded-full' : ''}`}
                />
              ) : (
                <ImageIcon size={15} />
              )}
            </button>
          </Tooltip>
          <input
            ref={nameRef}
            type="text"
            value={name}
            maxLength={MAX_NAME_LEN}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') handleSave();
            }}
            placeholder="Preset name (e.g. FNCS, ALGS)"
            className="glass-input flex-1 text-sm text-textPrimary placeholder:text-textMuted px-3 py-2 outline-none"
          />
        </div>

        <div className="glass-input flex items-center pl-3 pr-2 h-9">
          <Search size={14} className="text-textMuted shrink-0" />
          <input
            ref={channelInputRef}
            type="text"
            value={searchInput}
            onChange={(e) => setSearchInput(e.target.value)}
            onKeyDown={handleSearchKeyDown}
            onFocus={() => {
              if (blurTimer.current) clearTimeout(blurTimer.current);
              setSearchFocused(true);
              refreshFollowing();
            }}
            onBlur={() => {
              blurTimer.current = setTimeout(() => setSearchFocused(false), 150);
            }}
            placeholder="Add streamers..."
            className="bg-transparent border-none text-sm text-textPrimary placeholder:text-textMuted flex-1 px-2 outline-none h-full"
          />
          {isSearching ? (
            <Loader2 size={13} className="text-accent animate-spin shrink-0" />
          ) : searchInput ? (
            <button
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => reset()}
              className="text-textMuted hover:text-textPrimary transition-colors shrink-0"
            >
              <X size={13} />
            </button>
          ) : null}
        </div>
      </div>

      {/* One scroll region: search results (when searching) sit above the chosen
          channels. A single scroll container means nothing gets clipped. */}
      <div ref={listRef} className="flex-1 min-h-0 overflow-y-auto custom-scrollbar px-2.5 pb-2">
        {showResults && (
          // preventDefault on mousedown keeps the search input focused while you
          // click a result, so you can add several in a row without re-focusing.
          <div className="pb-2 mb-2 border-b border-white/[0.06]" onMouseDown={(e) => e.preventDefault()}>
            {followingItems.length > 0 && (
              <>
                <div className="px-1 pt-1 pb-1 flex items-center gap-1.5">
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
                      reason={gridRefusal(item.provider ?? 'twitch')}
                      onSelect={handleSelectChannel}
                      onHover={setHighlightIndex}
                    />
                  ))}
                </div>
              </>
            )}

            {query && (searchItems.length > 0 || isSearching) && (
              <>
                <div className="px-1 pt-2 pb-1 flex items-center gap-1.5">
                  <Search size={11} className="text-textMuted" />
                  <span className="text-[10px] font-bold uppercase tracking-wider text-textMuted">All channels</span>
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
                        reason={gridRefusal(item.provider ?? 'twitch')}
                        onSelect={handleSelectChannel}
                        onHover={setHighlightIndex}
                      />
                    );
                  })}
                </div>
              </>
            )}

            {visibleItems.length === 0 && (
              <div className="px-4 py-3 text-center">
                <span className="text-[11px] text-textMuted">
                  {query
                    ? isSearching
                      ? 'Searching Twitch...'
                      : `No channels found for "${searchInput}"`
                    : followedCount === 0
                      ? 'No live follows. Type to search.'
                      : 'Start typing to search any channel'}
                </span>
              </div>
            )}
          </div>
        )}

        <div className="text-[10px] font-bold uppercase tracking-wider text-textMuted px-1 pt-0.5 pb-1">
          In this preset{channels.length > 0 ? ` · ${channels.length}` : ''}
        </div>
        {channels.length === 0 ? (
          <div className="px-2 py-4 text-center text-[11px] text-textMuted">Search above to add streamers.</div>
        ) : (
          <div className="space-y-0.5 pb-1">
            {channels.map((ch) => (
              <div
                key={makeKey(ch.provider ?? 'twitch', ch.channelLogin)}
                className="group flex items-center gap-2.5 rounded-lg px-1.5 py-1.5 hover:bg-white/[0.05] transition-colors"
              >
                <img
                  src={ch.profileImageUrl || DEFAULT_AVATAR}
                  alt=""
                  onError={(e) => {
                    const img = e.currentTarget as HTMLImageElement;
                    if (img.src !== DEFAULT_AVATAR) img.src = DEFAULT_AVATAR;
                  }}
                  className="w-7 h-7 rounded-full object-cover bg-black/30 shrink-0"
                />
                <span className="flex-1 min-w-0 text-[13px] font-medium text-textPrimary truncate">
                  {ch.channelName || ch.channelLogin}
                </span>
                <Tooltip content="Remove" delay={200} side="bottom">
                  <button
                    onClick={() => removeChannel(ch)}
                    className="w-6 h-6 flex items-center justify-center rounded-md text-textMuted hover:text-red-400 hover:bg-red-500/10 transition-colors shrink-0"
                  >
                    <X size={13} />
                  </button>
                </Tooltip>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Footer */}
      <div className="flex items-center gap-2 px-2.5 py-2.5 border-t border-white/[0.06] shrink-0">
        <button
          onClick={onDone}
          className="flex-1 px-3 py-2 rounded-lg glass-button text-textSecondary hover:text-white text-[13px] font-semibold transition-colors"
        >
          Cancel
        </button>
        <button
          onClick={handleSave}
          disabled={!canSave}
          className="flex-1 px-3 py-2 rounded-lg glass-input text-accent hover:text-white text-[13px] font-bold transition-colors disabled:opacity-40 disabled:cursor-not-allowed flex items-center justify-center gap-1.5"
        >
          {saving ? <Loader2 size={14} className="animate-spin" /> : <Check size={14} />}
          {editingId ? 'Save' : 'Create'}
        </button>
      </div>
    </div>
  );
};

/* ------------------------------------------------------------------ */
/* Icon picker (editor sub-view)                                       */
/* ------------------------------------------------------------------ */

interface CategoryResult {
  id: string;
  name: string;
  box_art_url?: string;
}

/** Box / cover-art URLs carry {width}x{height} placeholders (or a trailing -WxH
 *  suffix); fill them with a small portrait size for the preset icon. */
function sizedBoxArt(url: string | undefined, w: number, h: number): string {
  if (!url) return '';
  if (url.includes('{width}') && url.includes('{height}')) {
    return url.replace('{width}', String(w)).replace('{height}', String(h));
  }
  return url.replace(/-\d+x\d+\.(jpg|jpeg|png)$/i, `-${w}x${h}.$1`);
}

/** Full-width editor sub-view for choosing a preset icon: either a Twitch game
 *  category's cover art (searched live) or one of the preset's channel avatars.
 *  Rendered in place of the fields so it never fights the panel for space. */
const PresetIconPicker: React.FC<{
  channels: MultiNookPresetChannel[];
  current?: MultiNookPresetIcon;
  onPick: (icon: MultiNookPresetIcon | null) => void;
  onBack: () => void;
}> = ({ channels, current, onPick, onBack }) => {
  const [tab, setTab] = useState<'game' | 'channel'>(current?.type === 'channel' ? 'channel' : 'game');
  const [gameQuery, setGameQuery] = useState('');
  const [gameResults, setGameResults] = useState<CategoryResult[]>([]);
  const [gameLoading, setGameLoading] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const gameInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (tab !== 'game') return;
    const t = setTimeout(() => gameInputRef.current?.focus({ preventScroll: true }), 60);
    return () => clearTimeout(t);
  }, [tab]);

  // Debounced category search via the existing Helix-backed command.
  useEffect(() => {
    const q = gameQuery.trim();
    if (!q) {
      setGameResults([]);
      setGameLoading(false);
      if (timer.current) clearTimeout(timer.current);
      return;
    }
    if (timer.current) clearTimeout(timer.current);
    setGameLoading(true);
    timer.current = setTimeout(async () => {
      try {
        const res = (await invoke('search_categories', { query: q, limit: 18 })) as CategoryResult[];
        setGameResults(Array.isArray(res) ? res : []);
      } catch (e) {
        Logger.error('[MultiNookPresets] category search failed', e);
        setGameResults([]);
      } finally {
        setGameLoading(false);
      }
    }, 300);
    return () => {
      if (timer.current) clearTimeout(timer.current);
    };
  }, [gameQuery]);

  // Active tab is a raised, accented pill inside the recessed glass-input track, so
  // the selected source reads clearly at a glance; inactive tabs stay flat + muted.
  const tabClass = (active: boolean) =>
    `flex-1 flex items-center justify-center gap-1.5 py-1.5 rounded-lg text-[12px] font-semibold transition-all ${
      active ? 'glass-button text-accent' : 'text-textMuted hover:text-textPrimary'
    }`;

  return (
    <div className="flex flex-col max-h-[80vh]">
      {/* Header */}
      <div className="flex items-center gap-2 px-2.5 pt-2.5 pb-2 shrink-0">
        <button
          onClick={onBack}
          className="w-6 h-6 flex items-center justify-center rounded-md glass-button text-textSecondary hover:text-white shrink-0"
        >
          <ArrowLeft size={13} />
        </button>
        <span className="flex-1 text-[10px] font-bold uppercase tracking-wider text-textMuted">Choose icon</span>
        {current && (
          <button
            onClick={() => onPick(null)}
            className="text-[11px] font-semibold text-textMuted hover:text-red-400 transition-colors"
          >
            Use default
          </button>
        )}
      </div>

      {/* Source tabs */}
      <div className="px-2.5 pb-2 shrink-0">
        <div className="flex items-center gap-1 p-1 rounded-lg glass-input">
          <button onClick={() => setTab('game')} className={tabClass(tab === 'game')}>
            <Gamepad2 size={13} /> Game
          </button>
          <button onClick={() => setTab('channel')} className={tabClass(tab === 'channel')}>
            <Users size={13} /> Channel
          </button>
        </div>
      </div>

      {tab === 'game' ? (
        <>
          <div className="px-2.5 pb-2 shrink-0">
            <div className="glass-input flex items-center pl-3 pr-2 h-9">
              <Search size={14} className="text-textMuted shrink-0" />
              <input
                ref={gameInputRef}
                type="text"
                value={gameQuery}
                onChange={(e) => setGameQuery(e.target.value)}
                placeholder="Search game categories..."
                className="bg-transparent border-none text-sm text-textPrimary placeholder:text-textMuted flex-1 px-2 outline-none h-full"
              />
              {gameLoading && <Loader2 size={13} className="text-accent animate-spin shrink-0" />}
            </div>
          </div>
          <div className="flex-1 min-h-0 overflow-y-auto custom-scrollbar px-2.5 pb-3">
            {gameResults.length > 0 ? (
              <div className="grid grid-cols-3 gap-2">
                {gameResults.map((g) => {
                  const art = sizedBoxArt(g.box_art_url, 144, 192);
                  const selected = current?.type === 'game' && current.label === g.name;
                  return (
                    <button
                      key={g.id}
                      onClick={() => onPick({ type: 'game', imageUrl: art, label: g.name })}
                      className={`group flex flex-col items-center gap-1 rounded-lg p-1 transition-colors ${
                        selected ? 'bg-accent/15' : 'hover:bg-white/[0.06]'
                      }`}
                    >
                      <div className="w-full aspect-[3/4] rounded-md overflow-hidden bg-black/30 ring-1 ring-white/10">
                        {art ? (
                          <img src={art} alt={g.name} className="w-full h-full object-cover" />
                        ) : (
                          <div className="w-full h-full flex items-center justify-center">
                            <Gamepad2 size={18} className="text-textMuted" />
                          </div>
                        )}
                      </div>
                      <span
                        className={`w-full text-[10px] font-medium text-center truncate ${
                          selected ? 'text-accent' : 'text-textSecondary group-hover:text-textPrimary'
                        }`}
                      >
                        {g.name}
                      </span>
                    </button>
                  );
                })}
              </div>
            ) : (
              <div className="px-4 py-6 text-center text-[11px] text-textMuted">
                {gameQuery.trim()
                  ? gameLoading
                    ? 'Searching...'
                    : `No categories for "${gameQuery}"`
                  : 'Search for a game to use its cover art.'}
              </div>
            )}
          </div>
        </>
      ) : (
        <div className="flex-1 min-h-0 overflow-y-auto custom-scrollbar px-2.5 pb-3">
          {channels.length > 0 ? (
            <div className="grid grid-cols-4 gap-2">
              {channels.map((ch) => {
                const avatar = ch.profileImageUrl || DEFAULT_AVATAR;
                const selected = current?.type === 'channel' && current.imageUrl === avatar;
                return (
                  <button
                    key={ch.channelLogin}
                    onClick={() => onPick({ type: 'channel', imageUrl: avatar, label: ch.channelName || ch.channelLogin })}
                    className={`group flex flex-col items-center gap-1 rounded-lg p-1.5 transition-colors ${
                      selected ? 'bg-accent/15' : 'hover:bg-white/[0.06]'
                    }`}
                  >
                    <img
                      src={avatar}
                      alt=""
                      onError={(e) => {
                        const img = e.currentTarget as HTMLImageElement;
                        if (img.src !== DEFAULT_AVATAR) img.src = DEFAULT_AVATAR;
                      }}
                      className={`w-11 h-11 rounded-full object-cover bg-black/30 ring-2 ${
                        selected ? 'ring-accent/50' : 'ring-transparent group-hover:ring-accent/30'
                      }`}
                    />
                    <span
                      className={`w-full text-[10px] font-medium text-center truncate ${
                        selected ? 'text-accent' : 'text-textSecondary group-hover:text-textPrimary'
                      }`}
                    >
                      {ch.channelName || ch.channelLogin}
                    </span>
                  </button>
                );
              })}
            </div>
          ) : (
            <div className="px-4 py-6 text-center text-[11px] text-textMuted">
              Add channels to the preset first, then pick one as the icon.
            </div>
          )}
        </div>
      )}
    </div>
  );
};

export default MultiNookPresets;
