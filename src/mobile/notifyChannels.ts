// Per-channel live-alert opt-out.
//
// Stored as an EXCLUSION set rather than an inclusion one, which is the whole
// product decision in one line: everyone keeps getting alerts for everything
// they follow, and an upgrade changes nothing until someone deliberately
// silences a channel. An inclusion list would have silently switched every
// existing user to receiving nothing.
//
// Kept out of notifications.ts on purpose: that module is about the OS surface
// (permission, channels, posting) and has no business reading the settings
// store.
//
// Rust owns the list and every rule about it (`set_live_alert`, Copy from
// Twitch, Undo). A press sends the state the screen showed it wanting, never a
// toggle, so a screen that has not caught up yet cannot flip a channel the
// wrong way. Every change goes through one chain, so presses apply in the
// order they were made.
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../stores/AppStore';
import { Logger } from '../utils/logger';
import type { TwitchStream } from '../types';

/** Twitch logins are already lowercase, but nothing guarantees a caller's is. */
function key(login: string): string {
  return login.trim().toLowerCase();
}

function mutedList(): string[] {
  return useAppStore.getState().settings.live_notifications?.muted_live_channels ?? [];
}

/** For display only: whether this Twitch login is on the muted list now. */
export function isChannelMuted(login: string): boolean {
  if (!login) return false;
  return mutedList().includes(key(login));
}

/** A channel as Rust keys it for live alerts. */
export interface AlertChannelRef {
  provider: string;
  userId: string;
  userLogin: string;
}

/** One followed channel in the alerts list: the Twitch fields plus Rust's answer. */
export interface AlertChannel extends TwitchStream {
  /** True while its live alerts are on; false when muted. */
  on?: boolean | null;
}

export interface AlertChannels {
  channels: AlertChannel[];
  /** Rust stopped at its page cap; whatever was dropped is stated in the UI. */
  truncated: boolean;
  /** Muted entries that match a listed channel; null when the follow list
   *  could not be read (signed out). */
  muted_count: number | null;
  /** The last Copy from Twitch this session, while it can still be undone
   *  (Rust keeps it 10 min, until the next copy or an account change). */
  last_copy?: CopyAnswer | null;
}

export interface CopyAnswer {
  copy_id: number;
  added: string[];
  bells_off: number;
  already: number;
  skipped: number;
  truncated: boolean;
}

export type CopyCode = 'signed-out' | 'wrong-account' | 'twitch-error' | 'busy' | 'stale' | 'save-failed';
export type CopyResult = { ok: true; answer: CopyAnswer } | { ok: false; code: CopyCode };
export type UndoResult = { ok: true; removed: number } | { ok: false; code: CopyCode };

const CODES: readonly string[] = ['signed-out', 'wrong-account', 'twitch-error', 'busy', 'stale', 'save-failed'];

/** The coded error Rust answered, or `twitch-error` for anything else. */
export function copyCode(err: unknown): CopyCode {
  const text = typeof err === 'string' ? err : err instanceof Error ? err.message : '';
  return CODES.includes(text) ? (text as CopyCode) : 'twitch-error';
}

let chain: Promise<unknown> = Promise.resolve();

/** Runs `work` after every change already asked for. */
function enqueue<T>(work: () => Promise<T>): Promise<T> {
  const run = chain.then(work);
  chain = run.catch(() => undefined);
  return run;
}

/** Sets one channel's live alerts. Answers the stored state (true = on), or
 *  null when Rust has no identity for the channel or the change failed; the
 *  caller then leaves what it shows to the store. */
export function setLiveAlert(channel: AlertChannelRef, on: boolean): Promise<boolean | null> {
  return enqueue(async () => {
    try {
      return await invoke<boolean | null>('set_live_alert', {
        provider: channel.provider,
        userId: channel.userId,
        userLogin: channel.userLogin,
        on,
      });
    } catch (err) {
      Logger.warn('[NotifyChannels] live alert change failed:', err);
      return null;
    }
  });
}

/** Every channel you follow on Twitch, with each one's state and the muted count. */
export function alertChannels(): Promise<AlertChannels> {
  return invoke<AlertChannels>('get_followed_channels_for_alerts');
}

/** Copy from Twitch: mutes every followed channel whose Twitch bell is off. */
export function copyTwitchBells(): Promise<CopyResult> {
  return enqueue(async () => {
    try {
      return { ok: true as const, answer: await invoke<CopyAnswer>('copy_twitch_live_mutes') };
    } catch (err) {
      Logger.warn('[NotifyChannels] copy from Twitch failed:', err);
      return { ok: false as const, code: copyCode(err) };
    }
  });
}

/** Undo one Copy from Twitch: turns alerts back on for what it added. */
export function undoTwitchBells(copyId: number): Promise<UndoResult> {
  return enqueue(async () => {
    try {
      return { ok: true as const, removed: await invoke<number>('undo_live_mutes', { copyId }) };
    } catch (err) {
      Logger.warn('[NotifyChannels] undo copy failed:', err);
      return { ok: false as const, code: copyCode(err) };
    }
  });
}
