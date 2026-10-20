import { describe, expect, it } from 'vitest';
import type { MentionEntry } from '../../types';
import { CHIP_LABEL, emptyWords, foldedWords, inboxTime, whereWords } from './inboxText';

const entry = (over: Partial<MentionEntry> = {}): MentionEntry => ({
  id: 'm',
  platform: 'twitch',
  room: 'xqc',
  channel_login: 'xqc',
  sender_id: '1',
  sender_login: 'viewer',
  sender_name: 'Viewer',
  text: '@you hi',
  kind: 'mention',
  at_ms: 0,
  read: false,
  removed: false,
  ...over,
});

describe('inbox words', () => {
  it('says a time the way the iPhone does', () => {
    const now = new Date(2026, 9, 19, 12, 0, 0).getTime();
    expect(inboxTime(now - 20_000, now)).toBe('now');
    expect(inboxTime(now - 5 * 60_000, now)).toBe('5m');
    expect(inboxTime(now - 3 * 3_600_000, now)).toBe('3h');
    expect(inboxTime(now - 2 * 86_400_000, now)).toBe('2d');
    expect(inboxTime(new Date(2026, 9, 3).getTime(), now)).toBe('Oct 3');
    expect(inboxTime(new Date(2025, 0, 9).getTime(), now)).toBe('Jan 9, 2025');
    // A clock a little ahead of the phone's still reads as now.
    expect(inboxTime(now + 5_000, now)).toBe('now');
  });

  it('counts a fold by what it holds', () => {
    expect(foldedWords(entry(), 6)).toBe('6 identical mentions');
    expect(foldedWords(entry({ kind: 'reply' }), 2)).toBe('2 identical replies');
    expect(foldedWords(entry({ kind: 'word' }), 3)).toBe('3 identical messages');
  });

  it('names where it was said, by the channel and not the room key', () => {
    expect(whereWords(entry())).toEqual({ lead: 'in', place: 'xqc' });
    expect(whereWords(entry({ room: 'kick:slug', channel_login: 'Slug' }))).toEqual({ lead: 'in', place: 'Slug' });
    expect(whereWords(entry({ room: 'kick:slug', channel_login: '' }))).toEqual({ lead: 'in', place: 'slug' });
  });

  it('labels every chip and every empty list', () => {
    for (const id of ['all', 'mentions', 'highlights', 'whispers'] as const) {
      expect(CHIP_LABEL[id]).toBeTruthy();
      expect(emptyWords(id).text).toMatch(/\.$/);
    }
  });
});
