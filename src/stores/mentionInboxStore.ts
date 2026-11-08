// The desktop's view of the inbox. Rust owns the inbox (mentions, whisper
// conversations, read state, what counts); the main window shows it in the
// Dynamic Island's notification history and MultiChat in its Mentions mode.
// This keeps only the event names, the counts this window last heard, the
// mute toggle, and which chats this window has on screen.

import { useEffect, useSyncExternalStore } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Logger } from '../utils/logger';
import type { InboxCounts } from '../types';

export const MENTION_INBOX_CHANGED_EVENT = 'mention-inbox-changed';
/** Rust decided to announce this entry now (the payload is the entry). */
export const MENTION_NOTIFY_EVENT = 'mention-notify';
/** Rust's inbox counts changed (`inbox_counts.rs`; the payload is the counts). */
export const INBOX_CHANGED_EVENT = 'inbox-changed';
/** One whisper conversation changed (`whisper_inbox.rs`). */
export const WHISPER_UPDATED_EVENT = 'whisper-conversation-updated';
/** Many whisper conversations changed, or a sync started or ended. */
export const WHISPER_ARCHIVE_CHANGED_EVENT = 'whisper-archive-changed';
/** A sender's picture landed in Rust's cache. */
export const CHANNEL_AVATARS_EVENT = 'channel-avatars';

// ---------------------------------------------------------------------------
// The inbox counts
// ---------------------------------------------------------------------------
//
// Rust's one count emitter publishes every count with a revision, and a view
// answer carries the same snapshot. Whichever arrives, the higher revision
// wins, so a slow answer never puts an older count back.

let counts: InboxCounts | null = null;
const countSubscribers = new Set<() => void>();
let countsListening = false;

/** The counts to keep: the newer revision wins. An equal revision keeps what
 *  is held, since a push carries every count and a seed only some. */
export function newerCounts(prev: InboxCounts | null, next: InboxCounts): InboxCounts {
  if (!prev || next.rev > prev.rev) return next;
  return prev;
}

/** Take counts from a push or a view answer. */
export function acceptInboxCounts(next: InboxCounts): void {
  const kept = newerCounts(counts, next);
  if (kept === counts) return;
  counts = kept;
  countSubscribers.forEach((fn) => fn());
}

function listenForCounts(): void {
  if (countsListening) return;
  countsListening = true;
  void listen<InboxCounts>(INBOX_CHANGED_EVENT, (event) => {
    if (event.payload) acceptInboxCounts(event.payload);
  });
}

const subscribeCounts = (fn: () => void) => {
  countSubscribers.add(fn);
  return () => {
    countSubscribers.delete(fn);
  };
};
const readCounts = () => counts;

/** The inbox counts this window last heard, or null before the first. */
export function useInboxCounts(): InboxCounts | null {
  useEffect(() => {
    listenForCounts();
  }, []);
  return useSyncExternalStore(subscribeCounts, readCounts, readCounts);
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
