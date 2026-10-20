// The phone's view of the whisper inbox. Rust owns the conversations
// (services/whisper_inbox.rs, the same store desktop's Whispers panel uses):
// the socket files every incoming whisper there, Rust marks them read, sends
// replies, and decides which whisper is worth a notification (posted from
// Rust itself, android_notify.rs, because wry pauses the page off screen).
// This module asks for the list and a thread, relays the screen's actions,
// and holds the one piece of page state Rust has no business with: what was
// typed here and is still on its way, or was refused.
import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useAppStore } from '../../stores/AppStore';
import { useMobileNavStore } from '../navStore';
import { closeInboxSheet } from '../mentions/mentionInbox';
import { Logger } from '../../utils/logger';
import type { WhisperConversation, WhisperList, WhisperListRow, WhisperUpdate } from '../../types';
import { failureReason, type PendingWhisper } from './whisperModel';

export type { WhisperListRow };

interface WhispersState {
  /** The account the list belongs to ('' signed out); the writes take it. */
  ownerId: string;
  rows: WhisperListRow[];
  unreadTotal: number;
  /** A list answer arrived at least once. */
  loaded: boolean;
  /** The last list read failed and there is nothing to show. */
  failed: boolean;
  /** Bumped on every change Rust reports, so an open thread re-reads. */
  revision: number;
  /** The newest change Rust reported, for the open thread to apply at once. */
  lastUpdate: WhisperUpdate | null;
  /** Typed here, not yet confirmed (or refused) by Twitch, per conversation. */
  pending: PendingWhisper[];
}

export const useWhispersStore = create<WhispersState>(() => ({
  ownerId: '',
  rows: [],
  unreadTotal: 0,
  loaded: false,
  failed: false,
  revision: 0,
  lastUpdate: null,
  pending: [],
}));

/** Re-read the list from Rust. */
export async function refreshWhisperList(): Promise<void> {
  try {
    const list = await invoke<WhisperList>('whisper_list');
    useWhispersStore.setState({
      ownerId: list.owner_id,
      rows: list.conversations,
      unreadTotal: list.unread_total,
      loaded: true,
      failed: false,
    });
  } catch (err) {
    Logger.warn('[Whispers] list failed:', err);
    useWhispersStore.setState((s) => ({ failed: !s.loaded }));
  }
}

/** One conversation with all its messages, or null when it is gone. */
export async function loadWhisperThread(key: string): Promise<WhisperConversation | null> {
  const ownerId = useWhispersStore.getState().ownerId;
  if (!ownerId) return null;
  return invoke<WhisperConversation | null>('whisper_thread', { ownerId, key });
}

/** The conversation is on screen and read: Rust zeroes its unread count. */
export async function markWhisperRead(key: string): Promise<void> {
  const { ownerId, rows } = useWhispersStore.getState();
  if (!ownerId) return;
  const row = rows.find((r) => r.key === key);
  if (row && row.unread_count > 0) {
    useWhispersStore.setState((s) => ({
      rows: s.rows.map((r) => (r.key === key ? { ...r, unread_count: 0 } : r)),
      unreadTotal: Math.max(0, s.unreadTotal - row.unread_count),
    }));
  }
  try {
    await invoke('whisper_mark_read', { ownerId, key });
  } catch (err) {
    Logger.warn('[Whispers] mark read failed:', err);
  }
}

// ---------------------------------------------------------------------------
// The conversation on screen
// ---------------------------------------------------------------------------
//
// Rust reads a whisper landing in the open conversation as read and does not
// notify for it. It only counts as open while the app is on screen: the page
// reports none the moment it is hidden, and the open one again when it comes
// back (marking it read, since whatever arrived meanwhile has now been seen).

let openKey: string | null = null;
let visibilityHooked = false;

function reportOpen(): void {
  const key = document.visibilityState === 'visible' ? openKey : null;
  invoke('whisper_set_active', { key }).catch((err) => Logger.debug('[Whispers] set active failed:', err));
  if (key) void markWhisperRead(key);
}

/** The conversation view came up (a key) or went away (null). */
export function setOpenWhisper(key: string | null): void {
  openKey = key;
  if (!visibilityHooked) {
    visibilityHooked = true;
    document.addEventListener('visibilitychange', reportOpen);
  }
  reportOpen();
}

// ---------------------------------------------------------------------------
// Sending
// ---------------------------------------------------------------------------

let localSeq = 0;

function setPending(localId: string, change: Partial<PendingWhisper> | null): void {
  useWhispersStore.setState((s) => ({
    pending: change
      ? s.pending.map((p) => (p.localId === localId ? { ...p, ...change } : p))
      : s.pending.filter((p) => p.localId !== localId),
  }));
}

async function deliver(localId: string, row: WhisperListRow, text: string): Promise<WhisperUpdate | null> {
  const ownerId = useWhispersStore.getState().ownerId;
  if (!ownerId) {
    setPending(localId, { status: 'failed', reason: 'You are not signed in to Twitch.' });
    return null;
  }
  try {
    const update = await invoke<WhisperUpdate>('whisper_send', {
      ownerId,
      target: {
        key: row.key,
        user_id: row.user_id,
        user_login: row.user_login,
        user_name: row.user_name,
        profile_image_url: row.profile_image_url,
      },
      text,
    });
    // Sent: the message now lives in Rust's conversation, so the pending copy
    // goes, and the list and the open thread take the update.
    setPending(localId, null);
    takeUpdate(update);
    return update;
  } catch (err) {
    Logger.warn('[Whispers] send failed:', err);
    setPending(localId, { status: 'failed', reason: failureReason(err) });
    return null;
  }
}

/** Send `text` to the conversation. Shown at once as sending; Twitch's answer
 *  turns it into the sent message or a failure with its reason. */
export function sendWhisper(row: WhisperListRow, text: string): void {
  const clean = text.trim();
  if (!clean) return;
  localSeq += 1;
  const pending: PendingWhisper = {
    localId: `local-${Date.now()}-${localSeq}`,
    key: row.key,
    text: clean,
    status: 'sending',
    atMs: Date.now(),
  };
  useWhispersStore.setState((s) => ({ pending: [...s.pending, pending] }));
  void deliver(pending.localId, row, clean);
}

export function retryWhisper(localId: string, row: WhisperListRow): void {
  const p = useWhispersStore.getState().pending.find((x) => x.localId === localId);
  if (!p || p.status !== 'failed') return;
  setPending(localId, { status: 'sending', reason: undefined, atMs: Date.now() });
  void deliver(localId, row, p.text);
}

export function discardWhisper(localId: string): void {
  setPending(localId, null);
}

// ---------------------------------------------------------------------------
// Rust's changes
// ---------------------------------------------------------------------------

function takeUpdate(update: WhisperUpdate): void {
  const s = useWhispersStore.getState();
  if (s.ownerId && update.owner_id !== s.ownerId) return;
  useWhispersStore.setState((st) => ({ revision: st.revision + 1, lastUpdate: update }));
  // The list is small and Rust sorts it; a re-read keeps one source of truth.
  void refreshWhisperList();
}

/** Open a conversation from a notification tap. Right after a cold start the
 *  account may not be known yet, so the list is read first. */
export async function openWhisperByKey(key: string): Promise<void> {
  if (!key) return;
  if (!useWhispersStore.getState().loaded) await refreshWhisperList();
  // Two sheets never stack: an open inbox gives way to the conversation.
  closeInboxSheet();
  useMobileNavStore.getState().openWhisper(key);
}

let installed = false;

/** Mounted once by the shell: the list and unread count, seeded now and kept
 *  current by Rust's events. Re-seeded on sign-in, since the inbox is per
 *  account. */
export function installWhisperInbox(): () => void {
  if (installed) return () => {};
  installed = true;
  void refreshWhisperList();
  const unlisteners: (() => void)[] = [];
  let dead = false;
  const hold = (p: Promise<() => void>) =>
    void p.then((u) => {
      if (dead) u();
      else unlisteners.push(u);
    });
  hold(listen<WhisperUpdate>('whisper-conversation-updated', (e) => takeUpdate(e.payload)));
  hold(listen<string>('whisper-archive-changed', () => {
    useWhispersStore.setState((s) => ({ revision: s.revision + 1, lastUpdate: null }));
    void refreshWhisperList();
  }));
  const unsubUser = useAppStore.subscribe((s, prev) => {
    if (s.currentUser?.user_id !== prev.currentUser?.user_id) {
      useWhispersStore.setState({ rows: [], unreadTotal: 0, loaded: false, pending: [], lastUpdate: null });
      void refreshWhisperList();
    }
  });
  return () => {
    dead = true;
    installed = false;
    unlisteners.forEach((u) => u());
    unsubUser();
  };
}
