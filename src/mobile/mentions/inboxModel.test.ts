import { describe, expect, it } from 'vitest';
import type { InboxItem, InboxViewPayload, MentionEntry, WhisperRowMeta } from '../../types';
import {
  DEFAULT_CHIPS,
  chipsOf,
  faceOfWhisper,
  mentionItems,
  mergePage,
  nextCounts,
  seenIdsOf,
  whisperName,
  whisperPreview,
} from './inboxModel';

const entry = (id: string): MentionEntry => ({
  id,
  platform: 'twitch',
  room: 'somechannel',
  channel_login: 'somechannel',
  sender_id: '99999999',
  sender_login: 'someviewer',
  sender_name: 'SomeViewer',
  text: 'hello there',
  kind: 'mention',
  at_ms: 0,
  read: false,
  removed: false,
});

const meta = (over: Partial<WhisperRowMeta> = {}): WhisperRowMeta => ({
  key: '1000000000',
  user_id: '1000000000',
  user_login: 'somefriend',
  user_name: 'SomeFriend',
  profile_image_url: null,
  color: '#8A2BE2',
  badges: [],
  last_message_timestamp: 5,
  last_text: 'see you later',
  last_parts: [],
  last_is_sent: false,
  unread_count: 1,
  archived: false,
  muted: false,
  ...over,
});

const mentionRow = (id: string, folded: string[] = []): InboxItem => ({
  kind: 'mention',
  row_id: id,
  entry: entry(id),
  folded: folded.map((f) => ({ id: f, at_ms: 0, read: false })),
  unread: true,
  avatar: null,
});

const whisperRow = (key: string): InboxItem => ({ kind: 'whisper', row_id: `w:${key}`, whisper: meta({ key }) });

const view = (rows: InboxItem[], over: Partial<InboxViewPayload> = {}): InboxViewPayload => ({
  rows,
  has_more: false,
  cursor: null,
  unread_total: 0,
  chips: [],
  counts_rev: 0,
  island_unread: 0,
  counts: { unread_total: 0, mentions: 0, whispers: 0, island: 0, rev: 0 },
  ...over,
});

describe('inbox rows', () => {
  it('marks a mention fold seen whole and a whisper conversation never', () => {
    expect(seenIdsOf(mentionRow('m1', ['m0']))).toEqual(['m1', 'm0']);
    expect(seenIdsOf(whisperRow('1000000000'))).toEqual([]);
  });

  it('adds the next page without repeating a row already shown', () => {
    const cur = view([mentionRow('m1'), whisperRow('1000000000')]);
    const next = view([whisperRow('1000000000'), mentionRow('m2')], { has_more: true, cursor: 'c2' });
    const merged = mergePage(cur, next);
    expect(merged.rows.map((r) => r.row_id)).toEqual(['m1', 'w:1000000000', 'm2']);
    expect(merged.cursor).toBe('c2');
    expect(merged.has_more).toBe(true);
  });

  it('tells mention rows apart from whisper rows', () => {
    expect(mentionItems([mentionRow('m1'), whisperRow('1000000000')]).map((r) => r.row_id)).toEqual(['m1']);
  });
});

describe('inbox chips', () => {
  it('shows Rust\'s chips once it answered, the three mention chips before', () => {
    expect(chipsOf(null)).toEqual(DEFAULT_CHIPS);
    expect(chipsOf(view([])).map((c) => c.id)).toEqual(['all', 'mentions', 'highlights']);
    const four = view([], {
      chips: [
        { id: 'all', unread: 3 },
        { id: 'mentions', unread: 1 },
        { id: 'highlights', unread: 0 },
        { id: 'whispers', unread: 2 },
      ],
    });
    expect(chipsOf(four).map((c) => c.id)).toEqual(['all', 'mentions', 'highlights', 'whispers']);
  });
});

describe('the counts revision', () => {
  it('never lets an older answer lower the count', () => {
    const shown = { unreadTotal: 4, countsRev: 10 };
    expect(nextCounts(shown, 3, 9)).toBeNull();
    expect(nextCounts(shown, 4, 10)).toBeNull();
    expect(nextCounts(shown, 2, 11)).toEqual({ unreadTotal: 2, countsRev: 11 });
  });
});

describe('whisper row words', () => {
  it('names the person and marks your own last line', () => {
    expect(whisperName(meta())).toBe('SomeFriend');
    expect(whisperName(meta({ user_name: '' }))).toBe('somefriend');
    expect(whisperName(meta({ user_name: '', user_login: '' }))).toBe('Someone');
    expect(whisperPreview(meta())).toBe('see you later');
    expect(whisperPreview(meta({ last_is_sent: true }))).toBe('You: see you later');
    expect(whisperPreview(meta({ last_text: '' }))).toBe('');
    expect(faceOfWhisper(meta())).toEqual({ name: 'SomeFriend', color: '#8A2BE2', kind: 'whisper' });
  });
});
