// The phone's link to the whisper store. Rust owns the conversations
// (services/whisper_inbox.rs, whisper_send.rs, whisper_sync.rs): it imports the
// history from Twitch, files every incoming whisper, marks conversations read
// here and on Twitch, sends, tracks where each send stands, and decides which
// whisper is worth a notification (posted from Rust itself, android_notify.rs,
// because wry pauses the page off screen). This module relays the conversation
// sheet's actions and hands Rust's changes to whatever is on screen. It holds
// no messages.
import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useAppStore } from '../../stores/AppStore';
import { useMobileNavStore } from '../navStore';
import { bumpInbox, inboxHooks, useMentionsStore, type InboxSurface } from '../mentions/mentionInbox';
import { Logger } from '../../utils/logger';
import { claimWhisper } from '../../utils/whisperClaim';
import type {
  WhisperArchiveChangedPayload,
  WhisperPagePayload,
  WhisperRowMeta,
  WhisperStatusPayload,
  WhisperUpdatePayload,
} from '../../types';
import { PAGE, failureReason } from './whisperModel';

export type WhisperStatus = WhisperStatusPayload;

interface WhispersState {
  /** What the inbox says about whispers beyond its rows. */
  status: WhisperStatus | null;
  /** Bumped when many conversations changed at once (a sync), so an open
   *  conversation reads its newest page again. */
  revision: number;
  /** The newest single change Rust reported, for the open conversation. */
  lastUpdate: WhisperUpdatePayload | null;
}

export const useWhispersStore = create<WhispersState>(() => ({
  status: null,
  revision: 0,
  lastUpdate: null,
}));

export async function refreshWhisperStatus(): Promise<void> {
  try {
    const status = await invoke<WhisperStatus>('whisper_status');
    useWhispersStore.setState({ status });
  } catch (err) {
    Logger.debug('[Whispers] status failed:', err);
  }
}

/** A page of one conversation, oldest first, before `cursor` when given. */
export function loadWhisperPage(key: string, cursor: string | null = null, limit = PAGE): Promise<WhisperPagePayload> {
  return invoke<WhisperPagePayload>('whisper_page', { key, cursor: cursor ?? undefined, limit });
}

/** Ask Rust for one more page of the conversation's history from Twitch;
 *  answers how many messages it added. */
export function loadOlderFromTwitch(key: string): Promise<number> {
  return invoke<number>('whisper_load_older', { key });
}

/** The conversation view came up (a key) or went away (null). Rust counts it
 *  as being read only while the app is in front and not in PiP (lifecycle.ts
 *  reports that), and reads it through as the claim starts counting. */
export function setOpenWhisper(key: string | null): void {
  claimWhisper(key);
}

/** Send `text`. Rust files it as Sending at once (the thread shows it from the
 *  update) and answers once Twitch has; a refusal before anything was filed
 *  comes back as its reason. */
export async function sendWhisper(meta: WhisperRowMeta, text: string): Promise<string | null> {
  try {
    await invoke('whisper_send', {
      target: {
        key: meta.key,
        user_id: meta.user_id,
        user_login: meta.user_login,
        user_name: meta.user_name,
        profile_image_url: meta.profile_image_url,
      },
      text,
    });
    return null;
  } catch (err) {
    Logger.warn('[Whispers] send failed:', err);
    return failureReason(err);
  }
}

export function retryWhisper(key: string, messageId: string): void {
  void invoke('whisper_retry', { key, messageId }).catch((err) => Logger.warn('[Whispers] retry failed:', err));
}

export function discardWhisper(key: string, messageId: string): void {
  void invoke('whisper_discard', { key, messageId }).catch((err) => Logger.warn('[Whispers] discard failed:', err));
}

export function markWhisperRead(key: string): void {
  void invoke('whisper_mark_read', { key }).catch((err) => Logger.warn('[Whispers] mark read failed:', err));
}

/** Archive the conversation, here and on Twitch. */
export async function archiveWhisper(key: string): Promise<void> {
  try {
    await invoke('whisper_archive', { key });
  } catch (err) {
    Logger.warn('[Whispers] archive failed:', err);
    useAppStore.getState().addToast('Could not archive that conversation.', 'error');
  }
}

/** Sync with Twitch now (the inbox screen's pull). */
export async function syncWhispersNow(): Promise<void> {
  try {
    await invoke('whisper_sync_now');
  } catch (err) {
    Logger.warn('[Whispers] sync failed:', err);
  }
  await refreshWhisperStatus();
}

/** Open a conversation on top of an inbox, so back leads to its list: an open
 *  inbox (the chat's sheet, or You > Inbox) turns into the thread, from a row,
 *  a notification tap or the arrival card. With none open, the card's chat
 *  opens its inbox sheet on it; a notification tap opens You > Inbox on it. */
export async function openWhisperByKey(key: string, surface?: InboxSurface): Promise<void> {
  if (!key) return;
  const nav = useMobileNavStore.getState();
  nav.openWhisper(key);
  if (useMentionsStore.getState().sheetIn || nav.mentionsOpen) return;
  if (surface) useMentionsStore.setState({ sheetIn: surface, arrival: null });
  else nav.setMentionsOpen(true);
}

function ownerMatches(ownerId: string | undefined): boolean {
  const mine = useAppStore.getState().currentUser?.user_id;
  return !mine || !ownerId || ownerId === mine;
}

let installed = false;

/** Mounted once by the shell: the status, and Rust's changes handed to the
 *  inbox and the open conversation. Reset on an account change, since
 *  whispers are per account. */
export function installWhisperInbox(): () => void {
  if (installed) return () => {};
  installed = true;
  inboxHooks.openWhisper = (key, surface) => void openWhisperByKey(key, surface);
  void refreshWhisperStatus();
  const unlisteners: (() => void)[] = [];
  let dead = false;
  const hold = (p: Promise<() => void>) =>
    void p.then((u) => {
      if (dead) u();
      else unlisteners.push(u);
    });
  hold(
    listen<WhisperUpdatePayload>('whisper-conversation-updated', (e) => {
      if (!e.payload || !ownerMatches(e.payload.owner_id)) return;
      useWhispersStore.setState({ lastUpdate: e.payload });
      bumpInbox();
    }),
  );
  hold(
    listen<WhisperArchiveChangedPayload | string>('whisper-archive-changed', (e) => {
      const p = e.payload;
      const owner = typeof p === 'string' ? p : p?.owner_id;
      if (!ownerMatches(owner)) return;
      const syncing = typeof p === 'string' ? undefined : p?.syncing;
      useWhispersStore.setState((s) => ({
        revision: s.revision + 1,
        lastUpdate: null,
        status: s.status && syncing !== undefined ? { ...s.status, syncing } : s.status,
      }));
      bumpInbox();
      void refreshWhisperStatus();
    }),
  );
  const unsubUser = useAppStore.subscribe((s, prev) => {
    if (s.currentUser?.user_id !== prev.currentUser?.user_id) {
      useWhispersStore.setState((st) => ({ status: null, lastUpdate: null, revision: st.revision + 1 }));
      void refreshWhisperStatus();
    }
  });
  return () => {
    dead = true;
    installed = false;
    inboxHooks.openWhisper = null;
    unlisteners.forEach((u) => u());
    unsubUser();
  };
}
