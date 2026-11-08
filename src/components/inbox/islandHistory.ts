// The island's history, newest first: its own notifications beside the inbox
// rows Rust's view hands it (mentions folded like the phones', and whisper
// conversations at their newest message). Pure, so the rules are tested on
// their own; MultiChat's list uses the row helpers too.
import type { DynamicIslandNotification, InboxItem, MentionEntry, MentionKind, WhisperRowMeta } from '../../types';

export interface FoldedMention {
  id: string;
  at_ms: number;
  read: boolean;
}

/** A mention row from Rust's inbox. Never cached: Rust owns it. */
export interface MentionItem {
  id: string;
  type: 'mention';
  /** The view's `row_id` (the head entry's id). */
  rowId: string;
  timestamp: number;
  read: boolean;
  data: MentionEntry;
  /** The older identical messages folded under it, newest first. */
  folded: FoldedMention[];
}

/** A whisper conversation, at its newest message. Never cached: Rust owns it. */
export interface WhisperRowItem {
  id: string;
  type: 'whisper_row';
  /** The view's `row_id` (`w:<key>`). */
  rowId: string;
  timestamp: number;
  read: boolean;
  data: WhisperRowMeta;
}

export type IslandItem = DynamicIslandNotification | MentionItem | WhisperRowItem;

export const toMentionItem = (entry: MentionEntry, folded: FoldedMention[] = [], unread?: boolean): MentionItem => ({
  id: `mention:${entry.id}`,
  type: 'mention',
  rowId: entry.id,
  timestamp: entry.at_ms,
  read: unread === undefined ? entry.read : !unread,
  data: entry,
  folded,
});

export const toWhisperItem = (whisper: WhisperRowMeta): WhisperRowItem => ({
  id: `w:${whisper.key}`,
  type: 'whisper_row',
  rowId: `w:${whisper.key}`,
  timestamp: whisper.last_message_timestamp,
  read: whisper.unread_count === 0,
  data: whisper,
});

export const itemFromRow = (row: InboxItem): MentionItem | WhisperRowItem => {
  if (row.kind === 'mention') {
    const item = toMentionItem(row.entry, row.folded, row.unread);
    return { ...item, rowId: row.row_id };
  }
  return { ...toWhisperItem(row.whisper), rowId: row.row_id };
};

/** Every inbox entry a mention row stands for: its head and the folded ones. */
export const mentionRowIds = (item: MentionItem): string[] => [item.data.id, ...item.folded.map((f) => f.id)];

/** Whether a row has anything unread. */
export const rowUnread = (row: InboxItem): boolean =>
  row.kind === 'mention' ? row.unread : row.whisper.unread_count > 0;

/** "N identical mentions", the fold's line under its row (the phones' words). */
export function foldedLine(kind: MentionKind, count: number): string {
  const noun = kind === 'reply' ? 'replies' : kind === 'word' ? 'messages' : 'mentions';
  return `${count} identical ${noun}`;
}

/** The oldest moment the loaded rows reach. */
export function oldestRowTime(rows: InboxItem[]): number {
  let oldest = Infinity;
  for (const row of rows) {
    if (row.kind === 'mention') {
      oldest = Math.min(oldest, row.entry.at_ms, ...row.folded.map((f) => f.at_ms));
    } else {
      oldest = Math.min(oldest, row.whisper.last_message_timestamp);
    }
  }
  return oldest;
}

/** One history, newest first. While older inbox rows are still unloaded the
 *  island's own entries stop at the oldest loaded row, so loading the next page
 *  only ever adds below the end and never pushes rows down. An old whisper
 *  notification never shows: the conversation's row is its history now. */
export function mergeIslandHistory(
  notifications: DynamicIslandNotification[],
  rows: InboxItem[],
  hasMore: boolean,
): IslandItem[] {
  const cutoff = hasMore && rows.length > 0 ? oldestRowTime(rows) : -Infinity;
  const own: IslandItem[] = notifications.filter((n) => n.type !== 'whisper' && n.timestamp >= cutoff);
  return [...own, ...rows.map(itemFromRow)].sort((a, b) => b.timestamp - a.timestamp);
}

/** These rows (all of them for `null`) read, at once, while Rust confirms. */
export function readRows(rows: InboxItem[], rowIds: string[] | null): InboxItem[] {
  const hit = (id: string) => rowIds === null || rowIds.includes(id);
  return rows.map((row) => {
    if (!hit(row.row_id)) return row;
    if (row.kind === 'mention') {
      return { ...row, unread: false, entry: { ...row.entry, read: true }, folded: row.folded.map((f) => ({ ...f, read: true })) };
    }
    return { ...row, whisper: { ...row.whisper, unread_count: 0 } };
  });
}

/** These rows gone, at once, while Rust confirms. */
export const dropRows = (rows: InboxItem[], rowIds: string[]): InboxItem[] =>
  rows.filter((row) => !rowIds.includes(row.row_id));

/** Whether a cached island notification is kept at start: not expired, and
 *  not an old whisper entry (whisper conversations are inbox rows now). */
export const keepCachedNotification = (n: DynamicIslandNotification, nowMs: number, expiryMs: number): boolean =>
  n.type !== 'whisper' && nowMs - n.timestamp < expiryMs;
