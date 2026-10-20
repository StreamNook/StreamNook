// The inbox's words, shared by the sheet, the arrival card and their tests.
// The same words as the iPhone's (SNKit MentionText, InboxText).
import type { InboxFilterId, MentionEntry } from '../../types';

/** The inbox's one spring (the sheet, the card, the tray leaving while you
 *  type): the desktop Dynamic Island's. */
export const INBOX_SPRING = { type: 'spring', stiffness: 360, damping: 32, mass: 0.9 } as const;

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

/** A row's time: "now", "5m", "3h", "2d", then the date ("Oct 3", with the
 *  year when it is not this year's). */
export function inboxTime(atMs: number, nowMs: number = Date.now()): string {
  const diff = Math.max(0, nowMs - atMs);
  if (diff < MINUTE) return 'now';
  if (diff < HOUR) return `${Math.floor(diff / MINUTE)}m`;
  if (diff < DAY) return `${Math.floor(diff / HOUR)}h`;
  if (diff < 7 * DAY) return `${Math.floor(diff / DAY)}d`;
  const d = new Date(atMs);
  const label = `${MONTHS[d.getMonth()]} ${d.getDate()}`;
  return d.getFullYear() === new Date(nowMs).getFullYear() ? label : `${label}, ${d.getFullYear()}`;
}

/** What a chip says. */
export const CHIP_LABEL: Record<InboxFilterId, string> = {
  all: 'All',
  mentions: 'Mentions',
  highlights: 'Highlights',
  whispers: 'Whispers',
};

/** The empty list's words for a chip. */
export function emptyWords(filter: InboxFilterId): { text: string; hint: string } {
  switch (filter) {
    case 'highlights':
      return { text: 'No highlights yet.', hint: 'Messages with one of your highlight phrases show up here.' };
    case 'whispers':
      return { text: 'No whispers yet.', hint: 'Whispers sent to you show up here.' };
    case 'mentions':
      return { text: 'No mentions yet.', hint: 'When someone mentions or replies to you in a chat, it shows up here.' };
    default:
      return { text: 'Nothing here yet.', hint: 'Mentions, replies and your highlight phrases show up here.' };
  }
}

/** "N identical mentions", the fold's line under its row. */
export function foldedWords(entry: MentionEntry, count: number): string {
  const noun = entry.kind === 'reply' ? 'replies' : entry.kind === 'word' ? 'messages' : 'mentions';
  return `${count} identical ${noun}`;
}

/** What follows the name: the chat it was said in, or how it reached you. */
export function whereWords(entry: MentionEntry): { lead: string; place: string | null } {
  if ((entry.kind as string) === 'whisper') return { lead: 'whispered', place: null };
  const i = entry.room.indexOf(':');
  return { lead: 'in', place: entry.channel_login || (i > 0 ? entry.room.slice(i + 1) : entry.room) };
}
