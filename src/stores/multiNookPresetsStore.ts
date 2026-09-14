import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from './AppStore';
import { usemultiNookStore } from './multiNookStore';
import { MultiNookPreset, MultiNookPresetChannel, MultiNookPresetIcon, MultiNookSlot } from '../types';
import { Logger } from '../utils/logger';
import { makeKey } from '../utils/providerKey';

/** Strip a live grid slot down to the lean, persistable preset-channel shape:
 *  identity, display metadata, and preferred quality, no transient view state. */
export function slotToPresetChannel(slot: MultiNookSlot): MultiNookPresetChannel {
  return {
    provider: slot.provider,
    channelLogin: slot.channelLogin,
    channelId: slot.channelId,
    channelName: slot.channelName,
    profileImageUrl: slot.profileImageUrl,
    quality: slot.quality,
  };
}

/** Drop duplicate channels, preserving first-seen order. Keyed by
 *  provider+channel, not a bare login: the same name on two platforms is two
 *  different channels, and case folding a bare login would also destroy a
 *  case-sensitive id (see utils/providerKey.ts). */
function dedupeChannels(channels: MultiNookPresetChannel[]): MultiNookPresetChannel[] {
  const seen = new Set<string>();
  return channels.filter((ch) => {
    if (!ch.channelLogin) return false;
    const key = makeKey(ch.provider ?? 'twitch', ch.channelLogin);
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

function newId(): string {
  return `preset-${Date.now()}-${Math.floor(Math.random() * 1e6)}`;
}

interface MultiNookPresetsState {
  presets: MultiNookPreset[];
  /** True once the in-memory list has been seeded from persisted settings. */
  hydrated: boolean;

  /** Seed (or re-seed) the list from settings.multi_nook_presets. Idempotent. */
  hydrate: () => void;
  /** Write the current list back to settings.json (and the in-memory AppStore). */
  persist: (presets: MultiNookPreset[]) => Promise<void>;

  createPreset: (name: string, channels: MultiNookPresetChannel[], icon?: MultiNookPresetIcon) => Promise<string>;
  updatePreset: (
    id: string,
    patch: { name?: string; channels?: MultiNookPresetChannel[]; icon?: MultiNookPresetIcon | null },
  ) => Promise<void>;
  renamePreset: (id: string, name: string) => Promise<void>;
  deletePreset: (id: string) => Promise<void>;
  duplicatePreset: (id: string) => Promise<void>;
  reorderPresets: (orderedIds: string[]) => Promise<void>;
  /** Capture the current MultiNook grid (visible + docked) as a new preset. */
  saveCurrentGridAsPreset: (name: string) => Promise<string | null>;
  /** Open a preset into the grid: 'replace' swaps the grid, 'append' merges in. */
  applyPreset: (id: string, mode: 'replace' | 'append') => Promise<void>;
  /** Stop the currently-equipped preset: close out all its streams, leave it saved. */
  stopActivePreset: () => Promise<void>;
}

export const useMultiNookPresetsStore = create<MultiNookPresetsState>((set, get) => ({
  presets: [],
  hydrated: false,

  hydrate: () => {
    const stored = useAppStore.getState().settings?.multi_nook_presets;
    set({
      presets: Array.isArray(stored) ? stored : [],
      hydrated: true,
    });
  },

  persist: async (presets) => {
    set({ presets });
    // Mirror multiNookStore.saveSlots: read the freshest settings, swap only this
    // one key, and write through. Unknown keys round-trip via the Rust `extra`
    // catch-all, so presets persist without a backend struct change.
    const currentSettings = useAppStore.getState().settings;
    const newSettings = { ...currentSettings, multi_nook_presets: presets };
    try {
      await invoke('save_settings', { settings: newSettings });
      useAppStore.setState({ settings: newSettings });
    } catch (e) {
      Logger.error('[MultiNookPresets] Failed to persist presets to settings', e);
    }
  },

  createPreset: async (name, channels, icon) => {
    const now = Date.now();
    const preset: MultiNookPreset = {
      id: newId(),
      name: name.trim() || 'Untitled preset',
      channels: dedupeChannels(channels),
      ...(icon ? { icon } : {}),
      createdAt: now,
      updatedAt: now,
    };
    await get().persist([...get().presets, preset]);
    return preset.id;
  },

  updatePreset: async (id, patch) => {
    const next = get().presets.map((p) =>
      p.id === id
        ? {
            ...p,
            ...(patch.name !== undefined ? { name: patch.name.trim() || p.name } : {}),
            ...(patch.channels !== undefined ? { channels: dedupeChannels(patch.channels) } : {}),
            // null clears the icon (back to the default avatar stack); an object sets it.
            ...(patch.icon !== undefined ? { icon: patch.icon ?? undefined } : {}),
            updatedAt: Date.now(),
          }
        : p,
    );
    await get().persist(next);
  },

  renamePreset: async (id, name) => {
    await get().updatePreset(id, { name });
  },

  deletePreset: async (id) => {
    // If the deleted preset is the one currently equipped, drop the tag so the
    // toolbar button falls back to the default icon (the grid keeps playing).
    if (usemultiNookStore.getState().activePresetId === id) {
      await usemultiNookStore.getState().setActivePresetId(null);
    }
    await get().persist(get().presets.filter((p) => p.id !== id));
  },

  duplicatePreset: async (id) => {
    const src = get().presets.find((p) => p.id === id);
    if (!src) return;
    const now = Date.now();
    const copy: MultiNookPreset = {
      id: newId(),
      name: `${src.name} (copy)`,
      channels: src.channels.map((ch) => ({ ...ch })),
      ...(src.icon ? { icon: { ...src.icon } } : {}),
      createdAt: now,
      updatedAt: now,
    };
    // Insert the copy directly after the original for predictable placement.
    const list = get().presets;
    const idx = list.findIndex((p) => p.id === id);
    const next = [...list.slice(0, idx + 1), copy, ...list.slice(idx + 1)];
    await get().persist(next);
  },

  reorderPresets: async (orderedIds) => {
    const byId = new Map(get().presets.map((p) => [p.id, p]));
    const next = orderedIds.map((id) => byId.get(id)).filter((p): p is MultiNookPreset => !!p);
    // Append any presets not present in the supplied order (defensive).
    for (const p of get().presets) {
      if (!orderedIds.includes(p.id)) next.push(p);
    }
    await get().persist(next);
  },

  saveCurrentGridAsPreset: async (name) => {
    const slots = usemultiNookStore.getState().slots;
    if (slots.length === 0) return null;
    const channels = dedupeChannels(slots.map(slotToPresetChannel));
    return get().createPreset(name, channels);
  },

  applyPreset: async (id, mode) => {
    const preset = get().presets.find((p) => p.id === id);
    if (!preset || preset.channels.length === 0) return;
    if (mode === 'replace') {
      // The grid becomes exactly this preset, so tag it as the equipped preset.
      await usemultiNookStore.getState().loadPresetChannels(preset.channels, 'replace', id);
    } else {
      // Append merges into whatever's already there, so the grid is no longer a
      // single preset: clear the equipped tag, then add the channels.
      await usemultiNookStore.getState().setActivePresetId(null);
      await usemultiNookStore.getState().loadPresetChannels(preset.channels, 'append');
    }
  },

  stopActivePreset: async () => {
    // Tear down the grid (stops all proxies) and clear the equipped tag. The
    // preset itself stays saved; this is "eject", not "delete".
    await usemultiNookStore.getState().clearAllSlots();
  },
}));
