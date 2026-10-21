import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Logger } from '../utils/logger';

/** A chat tab's live line from Rust (services/chat_dock_live.rs), by `makeKey`:
 *  docked chats on platforms other than Twitch, and every channel a tab strip
 *  declared through `useTabLive`. A key with no row has no answer yet. */
export interface TabLiveRow {
  live: boolean;
  viewer_count: number | null;
  category: string | null;
  title: string | null;
}

export const useTabLiveStore = create<{ live: Record<string, TabLiveRow> }>(() => ({ live: {} }));

let started = false;

/** Follow Rust's live lines in this window. Safe to call from every reader. */
export function startTabLive(): void {
  if (started) return;
  started = true;
  void listen<Record<string, TabLiveRow>>('chat-dock-live', (event) => useTabLiveStore.setState({ live: event.payload }));
  invoke<Record<string, TabLiveRow>>('get_chat_dock_live')
    .then((live) => useTabLiveStore.setState({ live }))
    .catch((e: unknown) => Logger.warn('[TabLive] load failed:', e));
}
