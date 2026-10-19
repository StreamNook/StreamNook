// Opening a mention (an inbox row or a notification click): Rust marks it read
// and hands back the entry, the chat for its room opens, and that chat scrolls
// to the message. The chat may not be up yet (the click can start the stream),
// so the jump waits here for a minute and the first chat showing that room
// takes it, once.

import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from './AppStore';
import { sliceLookupKey } from '../utils/providerKey';
import type { MentionEntry } from '../types';
import type { ProviderId } from '../types/providers';
import { Logger } from '../utils/logger';

const JUMP_EVENT = 'sn-mention-jump';
const HOLD_MS = 60_000;

export interface MentionJump {
  id: string;
  /** Room key (services/mention_inbox.rs `room_key`). */
  room: string;
  /** What `open_mention` returned; null when the entry is already gone. */
  entry: MentionEntry | null;
}

let pending: (MentionJump & { at: number }) | null = null;

/** The room key of a chat: bare Twitch login, `provider:slug` otherwise, all
 *  lowercased. It is the chat slice's key, which is the same fold Rust's
 *  `room_key` makes, so no second derivation lives here. */
export function mentionRoomKey(provider: ProviderId | string | undefined, channel: string): string {
  return sliceLookupKey((provider || 'twitch') as ProviderId, channel.replace(/^#/, ''));
}

export function requestMentionJump(jump: MentionJump): void {
  pending = { ...jump, room: jump.room.toLowerCase(), at: Date.now() };
  window.dispatchEvent(new CustomEvent(JUMP_EVENT));
}

/** The waiting jump, once, for a chat showing one of `rooms`. */
export function takeMentionJump(rooms: readonly string[]): MentionJump | null {
  if (!pending || rooms.length === 0) return null;
  if (Date.now() - pending.at > HOLD_MS) {
    pending = null;
    return null;
  }
  if (!rooms.includes(pending.room)) return null;
  const { id, room, entry } = pending;
  pending = null;
  return { id, room, entry };
}

/** Calls `fn` on each new request; returns the unsubscribe. */
export function onMentionJump(fn: () => void): () => void {
  window.addEventListener(JUMP_EVENT, fn);
  return () => window.removeEventListener(JUMP_EVENT, fn);
}

/** Main window: open a mention in the main chat. Marks it read in Rust, starts
 *  its channel unless that is already the one playing, and leaves the jump for
 *  the chat. */
export async function openMentionInMain(id: string, room: string): Promise<void> {
  let entry: MentionEntry | null = null;
  try {
    entry = await invoke<MentionEntry | null>('open_mention', { id });
  } catch (e) {
    Logger.debug('[Mentions] open_mention failed:', e);
  }
  const key = (entry?.room ?? room).toLowerCase();
  // Opening keeps the channel's own case (a YouTube id is case-sensitive).
  const open = entry?.open_key || room;
  requestMentionJump({ id, room: key, entry });
  const store = useAppStore.getState();
  const cur = store.currentStream;
  if (cur?.user_login && mentionRoomKey(cur.provider, cur.user_login) === key) return;
  const go = () => { void useAppStore.getState().startStream(open); };
  if (!store.isBooting) { go(); return; }
  const stop = useAppStore.subscribe((s) => {
    if (!s.isBooting) { stop(); go(); }
  });
}
