import { describe, expect, it } from 'vitest';
import { rowsFor, summaryLine, type UnbanRequest } from './unbanRequests';

const row = (id: string, channel: string, name: string, text: string): UnbanRequest => ({
  id,
  channel,
  broadcaster_id: '1',
  user_id: id,
  user_login: name.toLowerCase(),
  user_name: name,
  text,
  created_at_ms: 0,
  age: '4 minutes ago',
});

describe('unban requests strip', () => {
  it('keeps only the pane channel rows', () => {
    const rows = [row('a', 'chan', 'Gus', 'hi'), row('b', 'other', 'Amy', 'yo')];
    expect(rowsFor(rows, 'Chan').map((r) => r.id)).toEqual(['a']);
  });

  it('collapses to a count with the oldest request', () => {
    expect(summaryLine([row('a', 'c', 'Gus', 'I was joking')], false)).toBe('unban request · Gus: I was joking');
    expect(summaryLine([row('a', 'c', 'Gus', 'x'), row('b', 'c', 'Amy', 'y')], false)).toBe('unban requests · Gus: x');
    expect(summaryLine([row('a', 'c', 'Gus', 'x'), row('b', 'c', 'Amy', 'y')], true)).toBe('unban requests');
  });
});
