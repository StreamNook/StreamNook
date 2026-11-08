// The Whispers window's pure rules: how a pushed change lands in what is
// loaded, the words for each state Rust sets, and the composer's limit. Rust
// decides the states and enforces the limit; these only present them.
import type {
  StoredWhisperMessage,
  WhisperDelivery,
  WhisperListPayload,
  WhisperLiveNow,
  WhisperRowMeta,
  WhisperSyncOutcome,
  WhisperUpdatePayload,
} from '../../types';
import { deliveryOf } from '../../utils/whisperDelivery';
import { formatShortCount } from '../../utils/streamStats';

export type SortOption = 'recent' | 'name' | 'unread';

/** Rust's limit (`whisper_send::MAX_LEN`), in Unicode characters. */
export const MAX_LEN = 500;
/** The counter shows from here. */
export const COUNTER_FROM = 450;

/** Characters as Rust counts them (Unicode scalar values, not UTF-16 units). */
export const scalarLength = (text: string): number => Array.from(text).length;

/** Whether the trimmed text is over Rust's limit. */
export const isOverLimit = (text: string): boolean => scalarLength(text.trim()) > MAX_LEN;

/** "487/500" once the trimmed text nears the limit, else nothing. */
export function counterText(text: string): string | null {
  const n = scalarLength(text.trim());
  return n >= COUNTER_FROM ? `${n}/${MAX_LEN}` : null;
}

export const DELIVERY_WORDS: Record<WhisperDelivery, string> = {
  sending: 'Sending...',
  sent: 'Sent',
  unknown: 'Checking...',
  delivered: 'Delivered',
  not_delivered: 'Not delivered',
};

/** Where one of your sends stands, or nothing (theirs, or one sent before
 *  StreamNook tracked sends). */
export function deliveryLabel(m: StoredWhisperMessage): string {
  if (!m.is_sent || m.delivery === null || m.delivery === undefined) return '';
  const state = deliveryOf(m);
  return state ? DELIVERY_WORDS[state] : '';
}

/** Retry and Discard show only under a send Twitch did not deliver. */
export const canRetry = (m: StoredWhisperMessage): boolean =>
  m.is_sent && m.delivery !== null && m.delivery !== undefined && deliveryOf(m) === 'not_delivered';

/** The line under a whisper Twitch cut short, or nothing. */
export const cutWords = (m: StoredWhisperMessage): string | null =>
  typeof m.cut_to === 'number' ? `Twitch cut this whisper to ${m.cut_to} characters.` : null;

/** Oldest first, by time then id, the order Rust keeps. */
const byTime = (a: StoredWhisperMessage, b: StoredWhisperMessage): number =>
  a.timestamp - b.timestamp || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);

/** Both sets of messages once each, oldest first (an older page joining). */
export function mergeMessages(a: StoredWhisperMessage[], b: StoredWhisperMessage[]): StoredWhisperMessage[] {
  const byId = new Map<string, StoredWhisperMessage>();
  for (const m of a) byId.set(m.id, m);
  for (const m of b) byId.set(m.id, m);
  return [...byId.values()].sort(byTime);
}

/** One pushed change in the loaded messages: a removal (a discard, or our own
 *  copy giving way to Twitch's), then the message itself, replacing the copy
 *  with its id or joining in time order. */
export function applyMessageUpdate(messages: StoredWhisperMessage[], update: WhisperUpdatePayload): StoredWhisperMessage[] {
  let next = update.removed_message_id ? messages.filter((m) => m.id !== update.removed_message_id) : messages;
  const m = update.message;
  if (!m) return next;
  const at = next.findIndex((x) => x.id === m.id);
  if (at >= 0) {
    next = next.slice();
    next[at] = m;
    return next.sort(byTime);
  }
  return [...next, m].sort(byTime);
}

const byNewest = (a: WhisperRowMeta, b: WhisperRowMeta): number =>
  b.last_message_timestamp - a.last_message_timestamp || (a.key < b.key ? -1 : a.key > b.key ? 1 : 0);

/** One pushed change in the conversation list: the row moves to its key (a
 *  re-key replaces the old one), and an archived conversation leaves. Another
 *  account's change is ignored. */
export function applyRowUpdate(list: WhisperListPayload | null, update: WhisperUpdatePayload): WhisperListPayload | null {
  if (!list || (list.owner_id !== '' && update.owner_id !== list.owner_id)) return list;
  const rest = list.conversations.filter((r) => r.key !== update.key && r.key !== update.replaced_key);
  const conversations = update.meta.archived ? rest : [update.meta, ...rest].sort(byNewest);
  return { ...list, conversations };
}

/** A conversation that exists only here until its first send. */
export function draftRow(
  user: { id: string; login: string; display_name: string; profile_image_url?: string },
  nowMs: number = Date.now(),
): WhisperRowMeta {
  return {
    key: user.id,
    user_id: user.id,
    user_login: user.login,
    user_name: user.display_name,
    profile_image_url: user.profile_image_url ?? null,
    color: null,
    badges: [],
    last_message_timestamp: nowMs,
    last_text: '',
    last_parts: [],
    last_is_sent: false,
    unread_count: 0,
    archived: false,
    muted: false,
  };
}

/** The list with the draft first, unless Rust already has that person. */
export const mergeDraft = (rows: WhisperRowMeta[], draft: WhisperRowMeta | null): WhisperRowMeta[] =>
  draft && !rows.some((r) => r.key === draft.key || r.user_id === draft.user_id) ? [draft, ...rows] : rows;

/** The list in the order the viewer picked. */
export function sortRows(rows: WhisperRowMeta[], option: SortOption): WhisperRowMeta[] {
  const copy = rows.slice();
  if (option === 'name') copy.sort((a, b) => a.user_name.localeCompare(b.user_name));
  else if (option === 'unread') copy.sort((a, b) => b.unread_count - a.unread_count || byNewest(a, b));
  else copy.sort(byNewest);
  return copy;
}

/** Whose archive changed. Older cores sent the owner id as a bare string. */
export function archiveOwnerOf(payload: unknown): string | null {
  if (typeof payload === 'string') return payload || null;
  if (payload && typeof payload === 'object') {
    const owner = (payload as { owner_id?: unknown }).owner_id;
    if (typeof owner === 'string') return owner || null;
  }
  return null;
}

/** Whether a sync is running, from the archive event. */
export const archiveSyncing = (payload: unknown): boolean =>
  !!payload && typeof payload === 'object' && (payload as { syncing?: unknown }).syncing === true;

/** What the refresh button's answer says, or nothing worth saying. */
export function syncStatusWords(outcome: WhisperSyncOutcome): string | null {
  switch (outcome.outcome) {
    case 'done':
      return outcome.added > 0 ? `Added ${outcome.added} new message${outcome.added === 1 ? '' : 's'}` : 'Up to date';
    case 'partial':
      return 'Some older messages are still on Twitch';
    case 'still_running':
      return 'Still syncing...';
    case 'no_session':
      return 'Sign in to Twitch to load your whispers';
    case 'wrong_session':
      return 'Sign in to twitch.tv again to load your whispers';
    case 'failed':
      return `Could not refresh: ${outcome.reason}`;
    case 'superseded':
      return null;
  }
}

/** A message's or a row's time: the clock today, else the day. */
export function messageTime(atMs: number, nowMs: number): string {
  const d = new Date(atMs);
  if (d.toDateString() === new Date(nowMs).toDateString()) {
    return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  }
  return d.toLocaleDateString([], { month: 'short', day: 'numeric' });
}

/** Messages from one side this close together are one run (no gap between bubbles). */
export const RUN_GAP_MS = 5 * 60_000;
/** A pause this long inside one day gets its own time line. */
export const PAUSE_MS = 60 * 60_000;

const clock = (atMs: number): string =>
  new Date(atMs).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });

const startOfDay = (ms: number): number => {
  const d = new Date(ms);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
};

/** The line above a day's first message: "Today · 9:41 PM", "Yesterday",
 *  a weekday within the week, else the date (with the year when it is not
 *  this one). */
export function dayLabel(atMs: number, nowMs: number): string {
  const days = Math.round((startOfDay(nowMs) - startOfDay(atMs)) / 86_400_000);
  if (days <= 0) return `Today · ${clock(atMs)}`;
  if (days === 1) return 'Yesterday';
  const d = new Date(atMs);
  if (days < 7) return d.toLocaleDateString([], { weekday: 'long' });
  const sameYear = d.getFullYear() === new Date(nowMs).getFullYear();
  return d.toLocaleDateString([], sameYear ? { month: 'short', day: 'numeric' } : { month: 'short', day: 'numeric', year: 'numeric' });
}

export type ThreadItem =
  | { kind: 'day'; id: string; label: string }
  | {
      kind: 'message';
      id: string;
      message: StoredWhisperMessage;
      mine: boolean;
      /** First and last of a run of one side's messages. */
      first: boolean;
      last: boolean;
      /** Where the send stands shows under this one: the newest of your run,
       *  or any send that did not arrive (it needs its Retry). */
      showStatus: boolean;
    };

/** The thread as drawn: day lines (and a time line after a long pause), and
 *  each message knowing its place in its run. Oldest first. */
export function threadLayout(messages: StoredWhisperMessage[], nowMs: number): ThreadItem[] {
  const out: ThreadItem[] = [];
  let prev: StoredWhisperMessage | null = null;
  messages.forEach((m, i) => {
    const newDay = !prev || startOfDay(prev.timestamp) !== startOfDay(m.timestamp);
    const pause = !newDay && prev !== null && m.timestamp - prev.timestamp >= PAUSE_MS;
    if (newDay) out.push({ kind: 'day', id: `day-${m.id}`, label: dayLabel(m.timestamp, nowMs) });
    else if (pause) out.push({ kind: 'day', id: `pause-${m.id}`, label: clock(m.timestamp) });
    const next = messages[i + 1];
    const sameRun = (a: StoredWhisperMessage | null | undefined, b: StoredWhisperMessage | null | undefined) =>
      !!a && !!b && a.is_sent === b.is_sent && b.timestamp - a.timestamp < RUN_GAP_MS && startOfDay(a.timestamp) === startOfDay(b.timestamp);
    const first = newDay || pause || !sameRun(prev, m);
    const last = !sameRun(m, next) || (!!next && next.timestamp - m.timestamp >= PAUSE_MS);
    const tracked = m.is_sent && m.delivery !== null && m.delivery !== undefined;
    const showStatus = tracked && (last || deliveryOf(m) === 'not_delivered');
    out.push({ kind: 'message', id: m.id, message: m, mine: m.is_sent, first, last, showStatus });
    prev = m;
  });
  return out;
}

/** "Live · Just Chatting · 1.2K watching", or nothing while offline. */
export function liveLine(live: WhisperLiveNow | null | undefined): string | null {
  if (!live) return null;
  const parts = ['Live'];
  if (live.category.trim()) parts.push(live.category.trim());
  parts.push(`${formatShortCount(live.viewers)} watching`);
  return parts.join(' · ');
}

/** The header's quiet line about the sync: running, or when it last ended
 *  while this window watched (just "Synced with Twitch" before that). */
export function syncLine(syncing: boolean, syncedAtMs: number | null, nowMs: number): string {
  if (syncing) return 'Syncing with Twitch...';
  if (syncedAtMs === null) return 'Synced with Twitch';
  const minutes = Math.floor((nowMs - syncedAtMs) / 60_000);
  if (minutes < 1) return 'Synced with Twitch just now';
  if (minutes < 60) return `Synced with Twitch ${minutes} min ago`;
  return 'Synced with Twitch';
}

/** The list narrowed to names that contain the typed words (display only). */
export function filterRows(rows: WhisperRowMeta[], query: string): WhisperRowMeta[] {
  const q = query.trim().toLowerCase();
  if (!q) return rows;
  return rows.filter((r) => r.user_name.toLowerCase().includes(q) || r.user_login.toLowerCase().includes(q));
}
