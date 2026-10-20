// The phone's whisper words and the small pure rules the Whispers screens draw
// with. Rust owns the conversations (services/whisper_inbox.rs: what arrived,
// what is unread, what was sent, why a send failed); this only shapes them for
// the screen. The iPhone draws from the same rules (ios SNKit WhisperModels),
// in the same words.
import type { Whisper, WhisperConversation, WhisperListRow, WhisperUpdate } from '../../types';

/** The person a conversation is with, never empty. */
export function shownName(row: { user_name: string; user_login: string }): string {
  return row.user_name || row.user_login || 'Someone';
}

/** The list row's second line: the newest message, yours marked as yours. */
export function previewLine(row: WhisperListRow): string {
  if (!row.last_text) return '';
  return row.last_is_sent ? `You: ${row.last_text}` : row.last_text;
}

/** The composer's placeholder. */
export function composerPlaceholder(row: { user_name: string; user_login: string }): string {
  return `Whisper ${shownName(row)}`;
}

export const WHISPER_WORDS = {
  title: 'Whispers',
  entryDetail: 'Private messages on Twitch',
  empty: 'No whispers yet.',
  emptyHint: 'When someone whispers you on Twitch, the conversation shows up here.',
  failed: 'Whispers did not load.',
  failedHint: 'Pull down to try again.',
  threadEmpty: 'No messages yet.',
  sending: 'Sending',
  retry: 'Retry',
  discard: 'Discard',
  fallbackReason: 'Failed to send message. Please try again.',
} as const;

/** A row's time: "now", "5m", "3h", "2d", then the date, as Mentions shows it. */
export function rowTime(atMs: number, nowMs: number): string {
  const seconds = Math.max(0, Math.floor((nowMs - atMs) / 1000));
  if (seconds < 60) return 'now';
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
  if (seconds < 86_400) return `${Math.floor(seconds / 3600)}h`;
  if (seconds < 7 * 86_400) return `${Math.floor(seconds / 86_400)}d`;
  const d = new Date(atMs);
  const sameYear = d.getFullYear() === new Date(nowMs).getFullYear();
  return d.toLocaleDateString('en-US', sameYear ? { month: 'short', day: 'numeric' } : { month: 'short', day: 'numeric', year: 'numeric' });
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

/** A message typed here that Twitch has not confirmed: on its way, or refused
 *  with Rust's reason. Sent ones come back from Rust as real messages. */
export interface PendingWhisper {
  localId: string;
  key: string;
  text: string;
  status: 'sending' | 'failed';
  reason?: string;
  atMs: number;
}

/** One line of the conversation view. */
export type ThreadItem =
  | { kind: 'day'; id: string; label: string }
  | { kind: 'message'; id: string; message: Whisper; mine: boolean; last: boolean; time: string | null }
  | { kind: 'pending'; id: string; pending: PendingWhisper };

/** Messages close together from one side form a run; the run's last bubble
 *  carries the time. */
const RUN_GAP_MS = 5 * 60_000;

/** The conversation view's lines, oldest first: a heading per day, each
 *  message marked as the end of its run or not, then what is still pending. */
export function threadItems(
  messages: Whisper[],
  pending: PendingWhisper[],
  nowMs: number,
): ThreadItem[] {
  const out: ThreadItem[] = [];
  let lastDay = '';
  messages.forEach((m, i) => {
    const label = dayLabel(m.timestamp, nowMs);
    if (label !== lastDay) {
      out.push({ kind: 'day', id: `day-${m.timestamp}`, label });
      lastDay = label;
    }
    const next = messages[i + 1];
    const last =
      !next ||
      next.is_sent !== m.is_sent ||
      next.timestamp - m.timestamp > RUN_GAP_MS ||
      dayLabel(next.timestamp, nowMs) !== label;
    out.push({ kind: 'message', id: m.id, message: m, mine: m.is_sent, last, time: last ? clockTime(m.timestamp) : null });
  });
  if (pending.length > 0 && lastDay !== 'Today' && messages.length > 0) {
    out.push({ kind: 'day', id: 'day-pending', label: 'Today' });
  }
  for (const p of pending) out.push({ kind: 'pending', id: p.localId, pending: p });
  return out;
}

/** The thread after one of Rust's updates for it: the message added once,
 *  the meta taken. A different conversation's update leaves it alone. */
export function applyToThread(
  thread: WhisperConversation | null,
  key: string,
  update: WhisperUpdate,
): WhisperConversation | null {
  if (update.key !== key && update.replaced_key !== key) return thread;
  const base: WhisperConversation = thread ?? {
    user_id: update.meta.user_id,
    user_login: update.meta.user_login,
    user_name: update.meta.user_name,
    profile_image_url: update.meta.profile_image_url ?? undefined,
    messages: [],
    last_message_timestamp: update.meta.last_message_timestamp,
    unread_count: update.meta.unread_count,
  };
  const messages =
    update.message && !base.messages.some((m) => m.id === update.message?.id)
      ? [...base.messages, update.message].sort((a, b) => a.timestamp - b.timestamp)
      : base.messages;
  return {
    ...base,
    user_id: update.meta.user_id,
    user_login: update.meta.user_login,
    user_name: update.meta.user_name,
    profile_image_url: update.meta.profile_image_url ?? base.profile_image_url,
    last_message_timestamp: update.meta.last_message_timestamp,
    unread_count: update.meta.unread_count,
    messages,
  };
}

/** The reason under a failed bubble: Rust's sentence, or the plain fallback. */
export function failureReason(err: unknown): string {
  if (typeof err === 'string' && err.trim()) return err.trim();
  if (err instanceof Error && err.message.trim()) return err.message.trim();
  return WHISPER_WORDS.fallbackReason;
}
