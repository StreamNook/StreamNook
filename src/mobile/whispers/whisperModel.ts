// The phone's whisper conversation rules: how a page of messages from Rust
// (`whisper_page`) and Rust's live updates combine into the thread on screen,
// how it is laid out (a heading per day, runs of messages, the status line
// under our own sends), and the composer's limit. Rust owns the conversation
// (services/whisper_inbox.rs and whisper_send.rs: what arrived, what is unread,
// where a send stands); this only shapes it. The iPhone draws from the same
// rules (SNKit WhisperModels) in the same words.
import type { StoredWhisperMessage, WhisperDelivery, WhisperPagePayload, WhisperRowMeta, WhisperUpdatePayload } from '../../types';
import { DELIVERY_LABEL, cutLine, deliveryOf } from '../../utils/whisperDelivery';

/** Twitch's limit, counted in characters as Rust counts them (Unicode scalar
 *  values), for every recipient. */
export const MAX_WHISPER = 500;
/** The composer shows its count from here on. */
export const COUNTER_FROM = 450;
/** One `whisper_page` call. */
export const PAGE = 100;
/** The most a re-read asks for (Rust's largest page). */
export const REREAD_MAX = 200;

export const WHISPER_WORDS = {
  threadEmpty: 'No messages yet.',
  retry: 'Retry',
  discard: 'Discard',
  archive: 'Archive',
  deleted: 'Message deleted',
  edited: 'edited',
  showEarlier: 'Show earlier messages',
  olderOnTwitch: 'Load older messages from Twitch',
  loadingOlder: 'Loading older messages',
  olderFailed: 'Older messages did not load.',
  fallbackReason: 'Failed to send message. Please try again.',
} as const;

/** The person a conversation is with, never empty. */
export function shownName(row: { user_name: string; user_login: string }): string {
  return row.user_name || row.user_login || 'Someone';
}

/** The composer's placeholder. */
export function composerPlaceholder(row: { user_name: string; user_login: string }): string {
  return `Whisper ${shownName(row)}`;
}

/** Characters as Rust counts them: an emoji is one. */
export function scalarLength(text: string): number {
  return Array.from(text).length;
}

/** The count under the composer, once it is near the limit. */
export function counterText(length: number): string | null {
  return length >= COUNTER_FROM ? `${length}/${MAX_WHISPER}` : null;
}

/** Whether the draft can go: something to say, within the limit. */
export function canSend(draft: string): boolean {
  const text = draft.trim();
  return text.length > 0 && scalarLength(text) <= MAX_WHISPER;
}

/** The heading over a day of messages: "Today", "Yesterday", else the date. */
export function dayLabel(atMs: number, nowMs: number): string {
  const day = (ms: number) => {
    const d = new Date(ms);
    return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  };
  const diff = Math.round((day(nowMs) - day(atMs)) / 86_400_000);
  if (diff === 0) return 'Today';
  if (diff === 1) return 'Yesterday';
  const d = new Date(atMs);
  const sameYear = d.getFullYear() === new Date(nowMs).getFullYear();
  return d.toLocaleDateString(
    'en-US',
    sameYear ? { weekday: 'short', month: 'short', day: 'numeric' } : { month: 'short', day: 'numeric', year: 'numeric' },
  );
}

/** A clock time under a run of messages: "3:05 PM". */
export function clockTime(atMs: number): string {
  return new Date(atMs).toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' });
}

/** The reason under a refused send: Rust's sentence, or the plain fallback. */
export function failureReason(err: unknown): string {
  if (typeof err === 'string' && err.trim()) return err.trim();
  if (err instanceof Error && err.message.trim()) return err.message.trim();
  return WHISPER_WORDS.fallbackReason;
}

// ---------------------------------------------------------------------------
// The thread on screen
// ---------------------------------------------------------------------------

/** What the conversation view holds: the person, the loaded messages (oldest
 *  first, by Rust's order: time, then id), and how to reach older ones. */
export interface ThreadState {
  meta: WhisperRowMeta | null;
  messages: StoredWhisperMessage[];
  /** Asks `whisper_page` for the page before the oldest loaded message. */
  cursor: string | null;
  /** Older messages are stored here but not loaded yet. */
  hasMoreLocal: boolean;
  /** Older messages are still on Twitch (`whisper_load_older` brings a page). */
  olderOnTwitch: boolean;
}

function before(a: StoredWhisperMessage, b: StoredWhisperMessage): boolean {
  return a.timestamp < b.timestamp || (a.timestamp === b.timestamp && a.id < b.id);
}

function sorted(list: StoredWhisperMessage[]): StoredWhisperMessage[] {
  return [...list].sort((a, b) => (before(a, b) ? -1 : before(b, a) ? 1 : 0));
}

export function fromPage(p: WhisperPagePayload): ThreadState {
  return {
    meta: p.meta,
    messages: sorted(p.messages),
    cursor: p.cursor,
    hasMoreLocal: p.has_more_local,
    olderOnTwitch: p.older_on_twitch,
  };
}

/** An older page from `whisper_page`, added above what is loaded. */
export function withOlder(s: ThreadState, p: WhisperPagePayload): ThreadState {
  const have = new Set(s.messages.map((m) => m.id));
  return {
    meta: p.meta ?? s.meta,
    messages: sorted([...p.messages.filter((m) => !have.has(m.id)), ...s.messages]),
    cursor: p.messages.length ? p.cursor : s.cursor,
    hasMoreLocal: p.has_more_local,
    olderOnTwitch: p.older_on_twitch,
  };
}

/** The newest page read again (after a sync or an adoption): it replaces what
 *  it covers, and older messages loaded before stay above it. */
export function withRefresh(s: ThreadState, p: WhisperPagePayload): ThreadState {
  const fresh = sorted(p.messages);
  const oldest = fresh[0];
  if (!oldest) return fromPage(p);
  const kept = s.messages.filter((m) => before(m, oldest));
  if (kept.length === 0) return fromPage(p);
  return {
    meta: p.meta ?? s.meta,
    messages: [...kept, ...fresh],
    cursor: s.cursor,
    hasMoreLocal: s.hasMoreLocal,
    olderOnTwitch: p.older_on_twitch,
  };
}

function concerns(key: string, u: WhisperUpdatePayload): boolean {
  return u.key === key || u.replaced_key === key;
}

/** One of Rust's updates applied: the message added or replaced by id, a
 *  removed one dropped, the person's row taken. Another conversation's update
 *  leaves the thread as it is. */
export function withUpdate(s: ThreadState | null, key: string, u: WhisperUpdatePayload): ThreadState | null {
  if (!concerns(key, u)) return s;
  const base: ThreadState = s ?? { meta: u.meta, messages: [], cursor: null, hasMoreLocal: false, olderOnTwitch: false };
  let messages = base.messages;
  if (u.removed_message_id) messages = messages.filter((m) => m.id !== u.removed_message_id);
  const msg = u.message;
  if (msg) {
    const i = messages.findIndex((m) => m.id === msg.id);
    messages = i >= 0 ? messages.map((m, j) => (j === i ? msg : m)) : sorted([...messages, msg]);
  }
  return { ...base, meta: u.meta, messages };
}

const UNSETTLED: ReadonlySet<WhisperDelivery> = new Set(['sending', 'sent', 'unknown', 'not_delivered']);

/** An update brings one of our own messages under an id the thread does not
 *  hold while one of ours is still on its way: Rust may have swapped a local
 *  send for Twitch's copy, so the newest page is read again to drop the old
 *  one. */
export function updateNeedsReread(s: ThreadState | null, key: string, u: WhisperUpdatePayload): boolean {
  if (!s || !concerns(key, u) || !u.message?.is_sent) return false;
  const id = u.message.id;
  if (s.messages.some((m) => m.id === id)) return false;
  return s.messages.some((m) => {
    const d = m.is_sent ? deliveryOf(m) : null;
    return d !== null && UNSETTLED.has(d);
  });
}

/** The line under one of our sends: where it stands, why it failed, and
 *  whether Retry and Discard apply. */
export interface StatusLine {
  delivery: WhisperDelivery;
  label: string;
  failure: string | null;
  actions: boolean;
}

/** One line of the conversation view. */
export type ThreadItem =
  | { kind: 'day'; id: string; label: string }
  | {
      kind: 'message';
      id: string;
      message: StoredWhisperMessage;
      mine: boolean;
      /** The end of its run: the time shows under it. */
      last: boolean;
      time: string | null;
      status: StatusLine | null;
      /** Twitch shortened this whisper. */
      cut: string | null;
    };

/** Messages close together from one side form a run; the run's last bubble
 *  carries the time. */
const RUN_GAP_MS = 5 * 60_000;

function statusOf(m: StoredWhisperMessage, newestOwnId: string | null): StatusLine | null {
  if (!m.is_sent) return null;
  const d = deliveryOf(m);
  if (d === null) return null;
  const settled = d === 'sent' || d === 'delivered';
  if (settled && m.id !== newestOwnId) return null;
  return {
    delivery: d,
    label: DELIVERY_LABEL[d],
    failure: d === 'not_delivered' ? m.failure || WHISPER_WORDS.fallbackReason : null,
    actions: d === 'not_delivered',
  };
}

/** The conversation view's lines, oldest first: a heading per day, each
 *  message marked as the end of its run or not. Every send that is on its way
 *  or refused says so; of the settled ones, only the newest says Sent or
 *  Delivered. */
export function threadItems(messages: StoredWhisperMessage[], nowMs: number): ThreadItem[] {
  let newestOwnId: string | null = null;
  for (const m of messages) if (m.is_sent && deliveryOf(m) !== null) newestOwnId = m.id;
  const out: ThreadItem[] = [];
  let lastDay = '';
  messages.forEach((m, i) => {
    const label = dayLabel(m.timestamp, nowMs);
    if (label !== lastDay) {
      // Today's heading carries the time the day's talk began, as on desktop.
      const shown = label === 'Today' ? `Today · ${clockTime(m.timestamp)}` : label;
      out.push({ kind: 'day', id: `day-${m.timestamp}-${m.id}`, label: shown });
      lastDay = label;
    }
    const next = messages[i + 1];
    const status = statusOf(m, newestOwnId);
    const last =
      status !== null ||
      !next ||
      next.is_sent !== m.is_sent ||
      next.timestamp - m.timestamp > RUN_GAP_MS ||
      dayLabel(next.timestamp, nowMs) !== label;
    out.push({
      kind: 'message',
      id: m.id,
      message: m,
      mine: m.is_sent,
      last,
      time: last ? clockTime(m.timestamp) : null,
      status,
      cut: m.cut_to != null ? cutLine(m.cut_to) : null,
    });
  });
  return out;
}
