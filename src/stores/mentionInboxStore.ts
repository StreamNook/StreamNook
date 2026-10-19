// The desktop's view of the mentions inbox. Rust owns the inbox (entries,
// read state, what counts); this keeps only what the page draws from it: the
// unread total for the title bar, whether the Mentions screen is open, and
// which chats this window has on screen.

import { useEffect } from 'react';
import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { MentionPage } from '../types';
import { Logger } from '../utils/logger';

export const MENTION_INBOX_CHANGED_EVENT = 'mention-inbox-changed';

interface MentionInboxUi {
  open: boolean;
  unreadTotal: number;
  setOpen: (open: boolean) => void;
}

export const useMentionInboxStore = create<MentionInboxUi>((set) => ({
  open: false,
  unreadTotal: 0,
  setOpen: (open) => set({ open }),
}));

let unreadStarted = false;

/** The unread total, kept current from Rust's `mention-inbox-changed`. */
export function useMentionUnreadTotal(): number {
  useEffect(() => {
    if (unreadStarted) return;
    unreadStarted = true;
    void listen<{ unread_total: number }>(MENTION_INBOX_CHANGED_EVENT, (e) => {
      useMentionInboxStore.setState({ unreadTotal: e.payload?.unread_total ?? 0 });
    });
    invoke<MentionPage>('get_mention_inbox', { limit: 1 })
      .then((p) => useMentionInboxStore.setState({ unreadTotal: p.unread_total }))
      .catch((e) => Logger.debug('[Mentions] unread fetch failed:', e));
  }, []);
  return useMentionInboxStore((s) => s.unreadTotal);
}

/** Mute or unmute one chat's mentions (`mentions.muted_channels`). Rust owns
 *  the list: this is one toggle edit it applies to the stored set, and every
 *  window's store re-reads the result. */
export async function toggleMentionChatMuted(room: string): Promise<void> {
  const value = room.trim().toLowerCase();
  if (!value) return;
  try {
    await invoke('edit_settings_list', {
      path: 'mentions.muted_channels',
      edit: { op: 'toggle', value },
    });
  } catch (e) {
    Logger.warn('[Mentions] mute toggle failed:', e);
  }
}

// ---------------------------------------------------------------------------
// Which chats this window has on screen
// ---------------------------------------------------------------------------
//
// Rust reads mentions in an on-screen chat as read, and does not notify for
// them. A chat counts as on screen only while its window is focused and
// visible: a window behind a game is not being looked at. Each surface
// registers its rooms; the window sends their union, debounced. Losing focus
// sends the empty set at once, so the window gaining focus (which sends after
// the debounce) always lands last.

const SEND_DEBOUNCE_MS = 300;
const registered = new Map<symbol, readonly string[]>();
let lastSent = '';
let timer: ReturnType<typeof setTimeout> | null = null;
let listening = false;

function windowShown(): boolean {
  return document.visibilityState === 'visible' && document.hasFocus();
}

function send(): void {
  if (timer) { clearTimeout(timer); timer = null; }
  const rooms = windowShown()
    ? [...new Set([...registered.values()].flat().map((r) => r.toLowerCase()))].sort()
    : [];
  const key = rooms.join('|');
  if (key === lastSent) return;
  lastSent = key;
  invoke('set_visible_mention_rooms', { rooms }).catch((e) =>
    Logger.debug('[Mentions] set_visible_mention_rooms failed:', e),
  );
}

function schedule(): void {
  if (timer) clearTimeout(timer);
  timer = setTimeout(send, SEND_DEBOUNCE_MS);
}

function startListening(): void {
  if (listening) return;
  listening = true;
  const onChange = () => (windowShown() ? schedule() : send());
  window.addEventListener('focus', onChange);
  window.addEventListener('blur', onChange);
  document.addEventListener('visibilitychange', onChange);
}

/** Report `rooms` (room keys) as on screen while this component is mounted.
 *  Pass a stable string (`a|b`) or array; an empty value reports nothing. */
export function useVisibleMentionRooms(rooms: string): void {
  useEffect(() => {
    const list = rooms ? rooms.split('|').filter(Boolean) : [];
    if (list.length === 0) return;
    startListening();
    const token = Symbol('mention-rooms');
    registered.set(token, list);
    schedule();
    return () => {
      registered.delete(token);
      schedule();
    };
  }, [rooms]);
}
