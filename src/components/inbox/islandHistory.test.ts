import { describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));

import {
  dropRows,
  foldedLine,
  itemFromRow,
  keepCachedNotification,
  mentionRowIds,
  mergeIslandHistory,
  oldestRowTime,
  readRows,
  rowUnread,
  type MentionItem,
} from './islandHistory';
import { newerCounts } from '../../stores/mentionInboxStore';
import type { DynamicIslandNotification, InboxCounts, InboxItem, MentionEntry, WhisperRowMeta } from '../../types';

const entry = (id: string, atMs: number, read = false) =>
  ({ id, at_ms: atMs, read, kind: 'mention', room: 'twitch:chan', platform: 'twitch', text: 'hey' }) as unknown as MentionEntry;

const mentionRow = (id: string, atMs: number, folded: number[] = [], unread = true): InboxItem => ({
  kind: 'mention',
  row_id: id,
  entry: entry(id, atMs, !unread),
  folded: folded.map((t, i) => ({ id: `${id}-f${i}`, at_ms: t, read: !unread })),
  unread,
  avatar: null,
});

const whisper = (key: string, atMs: number, unread = 0): WhisperRowMeta => ({
  key,
  user_id: key,
  user_login: 'alpha',
  user_name: 'Alpha',
  profile_image_url: null,
  color: null,
  badges: [],
  last_message_timestamp: atMs,
  last_text: 'hi',
  last_parts: [],
  last_is_sent: false,
  unread_count: unread,
  archived: false,
  muted: false,
});

const whisperRow = (key: string, atMs: number, unread = 0): InboxItem => ({
  kind: 'whisper',
  row_id: `w:${key}`,
  whisper: whisper(key, atMs, unread),
});

const note = (id: string, type: string, timestamp: number) =>
  ({ id, type, timestamp, read: false, data: {} }) as unknown as DynamicIslandNotification;

describe('the island history', () => {
  it('puts whisper rows and folded mentions beside its own entries, newest first', () => {
    const items = mergeIslandHistory(
      [note('live-1', 'live', 500), note('live-2', 'live', 50)],
      [mentionRow('m1', 300, [100]), whisperRow('99999999', 200, 2)],
      false,
    );
    expect(items.map((i) => i.id)).toEqual(['live-1', 'mention:m1', 'w:99999999', 'live-2']);
  });

  it('stops its own entries at the oldest loaded row, folded entries included, while more are unloaded', () => {
    const rows = [mentionRow('m1', 300, [100]), whisperRow('99999999', 200)];
    expect(oldestRowTime(rows)).toBe(100);
    const items = mergeIslandHistory([note('live-1', 'live', 500), note('live-2', 'live', 50)], rows, true);
    expect(items.map((i) => i.id)).toEqual(['live-1', 'mention:m1', 'w:99999999']);
  });

  it('never shows an old whisper notification next to the conversation row', () => {
    const items = mergeIslandHistory([note('whisper-x', 'whisper', 400)], [whisperRow('99999999', 200)], false);
    expect(items.map((i) => i.id)).toEqual(['w:99999999']);
  });

  it('keeps Rust row ids and read state on the items', () => {
    const mention = itemFromRow(mentionRow('m1', 300, [100, 90], false)) as MentionItem;
    expect(mention.rowId).toBe('m1');
    expect(mention.read).toBe(true);
    expect(mentionRowIds(mention)).toEqual(['m1', 'm1-f0', 'm1-f1']);
    const w = itemFromRow(whisperRow('1000000000', 200, 3));
    expect(w.rowId).toBe('w:1000000000');
    expect(w.read).toBe(false);
  });

  it('reads and drops rows by row id', () => {
    const rows = [mentionRow('m1', 300, [100]), whisperRow('99999999', 200, 2)];
    const read = readRows(rows, ['w:99999999']);
    expect(rowUnread(read[0])).toBe(true);
    expect(rowUnread(read[1])).toBe(false);
    const all = readRows(rows, null);
    expect(all.some(rowUnread)).toBe(false);
    expect(all[0].kind === 'mention' && all[0].folded.every((f) => f.read)).toBe(true);
    expect(dropRows(rows, ['m1']).map((r) => r.row_id)).toEqual(['w:99999999']);
  });

  it('drops expired and whisper entries from the cache at start', () => {
    const week = 7 * 24 * 60 * 60 * 1000;
    expect(keepCachedNotification(note('a', 'live', 1000), 2000, week)).toBe(true);
    expect(keepCachedNotification(note('b', 'whisper', 1000), 2000, week)).toBe(false);
    expect(keepCachedNotification(note('c', 'live', 0), week + 1, week)).toBe(false);
  });

  it("words a fold like the phones'", () => {
    expect(foldedLine('mention', 3)).toBe('3 identical mentions');
    expect(foldedLine('reply', 2)).toBe('2 identical replies');
    expect(foldedLine('word', 4)).toBe('4 identical messages');
  });
});

describe('the inbox counts', () => {
  const counts = (rev: number, island: number): InboxCounts => ({ unread_total: island, mentions: 0, whispers: 0, island, rev });

  it('keeps the higher revision, so a slow seed never lowers a newer count', () => {
    const pushed = counts(10, 4);
    expect(newerCounts(null, counts(3, 1))).toEqual(counts(3, 1));
    expect(newerCounts(pushed, counts(9, 3))).toBe(pushed);
    expect(newerCounts(pushed, counts(10, 0))).toBe(pushed);
    expect(newerCounts(pushed, counts(11, 5))).toEqual(counts(11, 5));
  });
});
