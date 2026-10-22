// The user card's Moderate buttons, shared by the desktop card and the phone's
// ChatterTools. Rust decides every button and its words
// (services/chatter_card.rs); this turns a pressed button into the one request
// `moderate_chatter` takes, the chat command it prefills, and its Undo.

import { invoke } from '@tauri-apps/api/core';
import type { ModAction, ModRequest } from '../services/moderation';

export interface CardButton {
  id: string;
  label: string;
  kind: 'timeout' | 'ban' | 'unban' | 'warn';
  /** What `moderate_chatter` takes as `action`, passed through untouched. */
  action: Record<string, unknown>;
  reason: string | null;
  asks_reason: boolean;
  danger: boolean;
  spoken: string;
}

export interface HistoryEntry {
  id: string;
  kind: 'ban' | 'timeout' | 'unban' | 'untimeout' | 'warn' | 'acknowledged' | 'deleted' | 'comment';
  title: string;
  by: string | null;
  at_ms: number;
  when: string;
  quote: string | null;
}

export interface ModHistory {
  /** `not_loaded` is the quick first answer, asked with `history: false`. */
  status: 'available' | 'no_access' | 'signed_out' | 'failed' | 'not_loaded';
  entries: HistoryEntry[];
  current: { permanent: boolean; label: string } | null;
  note: string | null;
}

export interface LowTrustAction {
  id: 'monitor' | 'restrict' | 'clear';
  label: string;
  /** What `update_suspicious_user_status` takes as `status`. */
  status: string;
  spoken: string;
  /** "Now restricted", once Twitch took it. */
  done: string;
}

export interface LowTrust {
  status: 'monitored' | 'restricted' | 'none';
  /** "Restricted · Likely ban evader"; null when this session saw nothing. */
  line: string | null;
  actions: LowTrustAction[];
}

export interface CardTools {
  note: string | null;
  nickname: string | null;
  color: string | null;
  swatches: string[];
  moderate: { buttons: CardButton[]; custom: CardButton[]; history: ModHistory } | null;
  low_trust: LowTrust | null;
}

/** Twitch's longest warning or ban reason. */
export const REASON_MAX = 500;

/** History entries shown before "Show all N". */
export const HISTORY_PREVIEW = 3;

export function loadCardTools(args: {
  channel: string | null;
  channelId: string | null;
  userId: string;
  history?: boolean;
  /** The shell's own reading of the viewer's badges, for Rust to use until
   *  it has seen the room's USERSTATE. */
  viewerModerates?: boolean;
}): Promise<CardTools> {
  return invoke<CardTools>('get_chatter_card_tools', {
    provider: 'twitch',
    channel: args.channel,
    channelId: args.channelId,
    userId: args.userId,
    history: args.history ?? true,
    viewerModerates: args.viewerModerates ?? null,
  });
}

/** The reason sent with a button: what was typed, else the one saved on it. */
export function reasonFor(button: CardButton, typed?: string | null): string | null {
  const text = (typed ?? button.reason ?? '').trim().slice(0, REASON_MAX);
  return text || null;
}

/** The request `moderate_chatter` takes for a pressed button. */
export function requestFor(
  button: CardButton,
  target: { broadcasterId: string; userId: string; name: string },
  typed?: string | null,
): ModRequest {
  const reason = reasonFor(button, typed);
  return {
    provider: 'twitch',
    broadcasterId: target.broadcasterId,
    targetUserId: target.userId,
    targetName: target.name,
    action: button.action as unknown as ModAction,
    ...(reason ? { reason } : {}),
    // A card button is one press; Rust asks for a second when the viewer
    // turned on Ban confirmation.
    onePress: true,
  };
}

/** The button that reverses a ban or timeout the card just ran. */
export function undoFor(button: CardButton): CardButton {
  return {
    id: 'undo',
    label: 'Undo',
    kind: 'unban',
    action: { kind: 'unban', timeout: button.kind === 'timeout' },
    reason: null,
    asks_reason: false,
    danger: false,
    spoken: 'Undo',
  };
}

/** The chat command a Shift-press puts in the chat box instead of running the
 *  button, so the viewer can edit it first. */
export function commandFor(button: CardButton, login: string, typed?: string | null): string {
  const reason = reasonFor(button, typed);
  const tail = reason ? ` ${reason}` : '';
  const seconds = typeof button.action.seconds === 'number' ? button.action.seconds : null;
  switch (button.kind) {
    case 'timeout':
      return `/timeout ${login} ${seconds ?? 600}${tail}`;
    case 'ban':
      return `/ban ${login}${tail}`;
    case 'unban':
      return button.action.timeout === true ? `/untimeout ${login}` : `/unban ${login}`;
    case 'warn':
      return `/warn ${login}${tail || ' '}`;
  }
}

/** The history entries to draw: the first few, or all of them when opened. */
export function shownHistory(history: ModHistory | null | undefined, open: boolean): HistoryEntry[] {
  if (!history) return [];
  return open ? history.entries : history.entries.slice(0, HISTORY_PREVIEW);
}

/** Changes the chatter's low-trust treatment in the channel. */
export function setLowTrust(
  action: LowTrustAction,
  broadcasterId: string,
  userId: string,
  channel?: string,
): Promise<void> {
  return invoke<void>('update_suspicious_user_status', {
    broadcasterId,
    targetUserId: userId,
    status: action.status,
    channel: channel ?? null,
  });
}
