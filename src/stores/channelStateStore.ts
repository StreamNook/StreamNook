// Per-channel chat state owned by Rust (src-tauri/src/services/channel_state.rs):
// viewer count, Shared Viewership (who the channel is streaming with and the
// combined count), channel points (balance, custom name/icon, an available
// bonus claim) and pinned messages for every Twitch channel some window has chat
// open on. A window registers a watch per channel; Rust polls each section on
// its own cadence (viewers as one Helix batch for every watched channel) and
// emits `channel-state` only when the content changed. This store is the
// render model: it never fetches.
import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { ChannelState, ChannelStateUpdate } from '../types';
import { Logger } from '../utils/logger';

interface ChannelStateStore {
  channels: Map<string, ChannelState>;
  apply: (update: ChannelStateUpdate) => void;
  set: (state: ChannelState) => void;
  remove: (login: string) => void;
}

export const useChannelStateStore = create<ChannelStateStore>((set, get) => ({
  channels: new Map(),
  apply: (update) => {
    const login = update.login.toLowerCase();
    const current = get().channels.get(login);
    if (!current) return; // not watched from this window
    const next: ChannelState = { ...current };
    switch (update.section) {
      case 'viewers':
        next.viewer_count = update.viewer_count;
        next.viewers_at = update.at;
        break;
      case 'points':
        next.points = update.points;
        next.points_at = update.at;
        break;
      case 'pinned':
        next.pinned = update.pinned;
        next.pinned_at = update.at;
        break;
      case 'collab':
        next.collab = update.collab;
        next.collab_at = update.at;
        break;
    }
    const channels = new Map(get().channels);
    channels.set(login, next);
    set({ channels });
  },
  set: (state) => {
    const channels = new Map(get().channels);
    channels.set(state.login.toLowerCase(), state);
    set({ channels });
  },
  remove: (login) => {
    const channels = new Map(get().channels);
    channels.delete(login.toLowerCase());
    set({ channels });
  },
}));

// How many watchers in THIS window hold each channel. Rust counts watches
// across windows, but this window's copy of the state must outlive every one
// of its own watchers, not just the first to leave: a chat pane and the
// MultiChat viewer total can watch the same channel.
const localWatchers = new Map<string, number>();

let listening = false;
function ensureListener() {
  if (listening) return;
  listening = true;
  void listen<ChannelStateUpdate>('channel-state', (event) => {
    useChannelStateStore.getState().apply(event.payload);
  });
}

/** Start watching `login` from this window. Idempotent per call pair with
 *  `unwatchChannel`; Rust refcounts across windows. */
export async function watchChannel(login: string, channelId: string): Promise<void> {
  ensureListener();
  const key = login.toLowerCase();
  localWatchers.set(key, (localWatchers.get(key) ?? 0) + 1);
  try {
    const state = await invoke<ChannelState>('watch_channel_state', { login: key, channelId });
    // Unwatched again while this was in flight: nothing here wants it now.
    if (localWatchers.has(key)) useChannelStateStore.getState().set(state);
  } catch (e) {
    Logger.warn('[ChannelState] watch failed:', e);
  }
}

export async function unwatchChannel(login: string): Promise<void> {
  const key = login.toLowerCase();
  const left = (localWatchers.get(key) ?? 1) - 1;
  if (left > 0) {
    localWatchers.set(key, left);
  } else {
    localWatchers.delete(key);
    useChannelStateStore.getState().remove(key);
  }
  try {
    await invoke('unwatch_channel_state', { login: key });
  } catch {
    /* window closing */
  }
}

/** Ask Rust to refresh one section now (after a pin, a claim, a spend). */
export function refreshChannelState(login: string, section: 'viewers' | 'points' | 'pinned'): Promise<void> {
  return invoke<void>('refresh_channel_state', { login: login.toLowerCase(), section }).catch((e: unknown) => {
    Logger.debug('[ChannelState] refresh failed:', e);
  });
}

/** The current state for `login`, or null when not watched. Stable object
 *  identity between updates, so it is safe as an effect dependency. */
export function useChannelState(login: string | null | undefined): ChannelState | null {
  const key = login ? login.toLowerCase() : null;
  return useChannelStateStore((s) => (key ? s.channels.get(key) ?? null : null));
}
