// Per-channel chat state owned by Rust (src-tauri/src/services/channel_state.rs):
// the live broadcast (viewer count, start, title, category), Shared Viewership (who the channel is streaming with and the
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
    // A first watch has Rust fetch at once, and that answer can land before
    // the watch call itself returns; dropping it left a channel with no live
    // line until its numbers next changed. A channel this window is watching
    // takes the update even before its first state arrives.
    const current = get().channels.get(login) ?? (localWatchers.has(login) ? emptyState(login) : null);
    if (!current) return; // not watched from this window
    const next: ChannelState = { ...current };
    switch (update.section) {
      case 'viewers':
        next.viewer_count = update.viewer_count;
        next.started_at = update.started_at;
        next.title = update.title;
        next.game_name = update.game_name;
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
    const key = state.login.toLowerCase();
    const prev = get().channels.get(key);
    const channels = new Map(get().channels);
    // The watch call's answer can be older than an update that beat it here.
    channels.set(key, prev ? newestSections(prev, state) : state);
    set({ channels });
  },
  remove: (login) => {
    const channels = new Map(get().channels);
    channels.delete(login.toLowerCase());
    set({ channels });
  },
}));

function emptyState(login: string): ChannelState {
  return {
    login,
    channel_id: '',
    viewer_count: null,
    started_at: null,
    title: null,
    game_name: null,
    viewers_at: null,
    points: null,
    points_at: null,
    pinned: [],
    pinned_at: null,
    collab: null,
    collab_at: null,
  };
}

const isNewer = (a: number | null, b: number | null) => (a ?? -1) > (b ?? -1);

/** `incoming`, keeping each section `prev` holds a newer copy of. */
export function newestSections(prev: ChannelState, incoming: ChannelState): ChannelState {
  const out: ChannelState = { ...incoming };
  if (isNewer(prev.viewers_at, incoming.viewers_at)) {
    out.viewer_count = prev.viewer_count;
    out.started_at = prev.started_at;
    out.title = prev.title;
    out.game_name = prev.game_name;
    out.viewers_at = prev.viewers_at;
  }
  if (isNewer(prev.points_at, incoming.points_at)) {
    out.points = prev.points;
    out.points_at = prev.points_at;
  }
  if (isNewer(prev.pinned_at, incoming.pinned_at)) {
    out.pinned = prev.pinned;
    out.pinned_at = prev.pinned_at;
  }
  if (isNewer(prev.collab_at, incoming.collab_at)) {
    out.collab = prev.collab;
    out.collab_at = prev.collab_at;
  }
  return out;
}

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
 *  `unwatchChannel` (pass the same `liveOnly`); Rust refcounts across windows.
 *  `liveOnly` asks for the live section alone (viewers, start, title,
 *  category), which rides one batched poll for every channel. */
export async function watchChannel(login: string, channelId: string, liveOnly = false): Promise<void> {
  ensureListener();
  const key = login.toLowerCase();
  localWatchers.set(key, (localWatchers.get(key) ?? 0) + 1);
  try {
    const state = await invoke<ChannelState>('watch_channel_state', { login: key, channelId, liveOnly });
    // Unwatched again while this was in flight: nothing here wants it now.
    if (localWatchers.has(key)) useChannelStateStore.getState().set(state);
  } catch (e) {
    Logger.warn('[ChannelState] watch failed:', e);
  }
}

export async function unwatchChannel(login: string, liveOnly = false): Promise<void> {
  const key = login.toLowerCase();
  const left = (localWatchers.get(key) ?? 1) - 1;
  if (left > 0) {
    localWatchers.set(key, left);
  } else {
    localWatchers.delete(key);
    useChannelStateStore.getState().remove(key);
  }
  try {
    await invoke('unwatch_channel_state', { login: key, liveOnly });
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
