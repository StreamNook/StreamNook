// The cleared chat's count (SN-40): only rows the list shows and that are not
// Twitch's own notices count as news.
import { describe, expect, it } from 'vitest';
import { countNew, countsAsNew } from './countableRow';

const row = (tags: Record<string, string> | Map<string, string>, extra: object = {}) => ({
  id: 'm1',
  provider: 'twitch',
  content: 'hi',
  tags,
  ...extra,
});

describe('countsAsNew', () => {
  it('counts an ordinary chat row', () => {
    expect(countsAsNew(row(new Map([['user-id', '42']])), undefined)).toBe(true);
    expect(countsAsNew(row({ 'user-id': '42' }), [])).toBe(true);
  });

  it('never counts a system line', () => {
    expect(countsAsNew(row(new Map([['user-id', 'tw-system']])), undefined)).toBe(false);
    expect(countsAsNew(row({ 'user-id': 'tw-system' }), undefined)).toBe(false);
  });

  it('never counts a row a setting hides from the list', () => {
    const raid = row(new Map([['msg-id', 'raid']]), { metadata: { msg_type: 'raid' } });
    expect(countsAsNew(raid, undefined)).toBe(true);
    expect(countsAsNew(raid, ['twitch:raid'])).toBe(false);
  });

  it('counts a raw line, never an empty one', () => {
    expect(countsAsNew('@id=1 :a!a@a PRIVMSG #x :hi', undefined)).toBe(true);
    expect(countsAsNew('', undefined)).toBe(false);
    expect(countsAsNew(null, undefined)).toBe(false);
  });

  it('totals a batch', () => {
    const rows = [row({ 'user-id': '1' }), row({ 'user-id': 'tw-system' }), row({ 'user-id': '2' })];
    expect(countNew(rows, undefined)).toBe(2);
  });
});
