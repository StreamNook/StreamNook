// AutoMod held messages as rows of the phone's chat, where they arrived.
//
// Rust owns the queue and its words (services/automod_queue.rs); the AutoMod
// store mirrors it. This turns each held or decided message into a row the
// shared list draws (HeldMessageRow) and places it by time among the room's
// messages, so a moderator sees it in context and it folds in place once
// someone allows or denies it.
import type { BackendChatMessage } from '../../services/twitchChat';
import type { DecidedHeld, HeldMessage, HeldRowData } from '../../stores/automodStore';

export type { HeldRowData };

/** The row id a held message lives under, apart from the real message
 *  Twitch delivers when it is allowed. */
export const heldRowId = (messageId: string) => `held-${messageId}`;

export const isHeldRowId = (id: string | null | undefined) => !!id && id.startsWith('held-');

/** A held message as a list row. */
export function heldRowMessage(data: HeldRowData): BackendChatMessage {
  const { held } = data;
  const id = heldRowId(held.message_id);
  return {
    id,
    timestamp: String(held.held_at_ms),
    username: held.user_login,
    display_name: held.user_name || held.user_login,
    color: undefined,
    badges: [],
    content: held.text,
    segments: [{ type: 'text', content: held.text }],
    is_action: false,
    is_first_message: false,
    is_mentioned: false,
    is_from_shared_chat: false,
    tags: new Map([
      ['id', id],
      ['user-id', held.user_id],
      ['tmi-sent-ts', String(held.held_at_ms)],
    ]),
    automodHeld: data,
  } as unknown as BackendChatMessage;
}

/** When a row was sent, in ms, or null when it does not say. */
export function sentAt(message: string | BackendChatMessage): number | null {
  if (typeof message === 'string') {
    const ts = message.match(/(?:^@|;)tmi-sent-ts=(\d+)/);
    return ts ? Number(ts[1]) : null;
  }
  const tag = message.tags instanceof Map ? message.tags.get('tmi-sent-ts') : undefined;
  if (tag && /^\d+$/.test(tag)) return Number(tag);
  const raw = message.timestamp;
  if (typeof raw === 'string' && /^\d+$/.test(raw)) return Number(raw);
  const parsed = typeof raw === 'string' ? Date.parse(raw) : NaN;
  return Number.isNaN(parsed) ? null : parsed;
}

/** The rows for one room: every waiting message, and every decided one the
 *  room still shows (an allowed message's real row arrives on its own). */
export function heldRows(held: HeldMessage[] | undefined, decided: DecidedHeld[] | undefined): HeldRowData[] {
  const rows: HeldRowData[] = (held ?? []).map((h) => ({ held: h, line: null, by: null }));
  for (const d of decided ?? []) {
    rows.push({ held: d.held, line: d.line ?? d.held.label ?? 'Held message decided', by: d.moderator });
  }
  return rows.sort((a, b) => a.held.held_at_ms - b.held.held_at_ms);
}

/** `messages` with the held rows placed by time: each after the last message
 *  sent at or before it was held. Rows older than everything shown are left
 *  out (they belong to scrolled-away history); a list with no times gets
 *  them at the end. */
export function mergeHeld(
  messages: (string | BackendChatMessage)[],
  rows: BackendChatMessage[],
): (string | BackendChatMessage)[] {
  if (rows.length === 0) return messages;
  const times = messages.map(sentAt);
  const known = times.filter((t): t is number => t != null);
  if (known.length === 0) return [...messages, ...rows];
  const oldest = Math.min(...known);
  const out: (string | BackendChatMessage)[] = [];
  let r = 0;
  const rowTime = (row: BackendChatMessage) => sentAt(row) ?? Number.MAX_SAFE_INTEGER;
  while (r < rows.length && rowTime(rows[r]) < oldest) r += 1;
  messages.forEach((message, i) => {
    out.push(message);
    const t = times[i];
    if (t == null) return;
    // A held row goes after the last message at or before its time.
    const nextTime = times.slice(i + 1).find((x): x is number => x != null);
    while (r < rows.length && rowTime(rows[r]) >= t && (nextTime == null || rowTime(rows[r]) < nextTime)) {
      out.push(rows[r]);
      r += 1;
    }
  });
  while (r < rows.length) out.push(rows[r++]);
  return out;
}
