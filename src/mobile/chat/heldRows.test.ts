import { describe, expect, it } from 'vitest';
import type { BackendChatMessage } from '../../services/twitchChat';
import type { HeldMessage } from '../../stores/automodStore';
import { heldRowMessage, heldRows, isHeldRowId, mergeHeld, sentAt } from './heldRows';

const held = (id: string, at: number, status = 'held'): HeldMessage => ({
  message_id: id,
  channel: 'zackrawrr',
  broadcaster_id: '1',
  user_id: '42',
  user_login: 'grumpy_gus',
  user_name: 'Grumpy_Gus',
  text: 'this game is broken',
  reason: 'automod',
  held_at_ms: at,
  status,
  label: 'Held by AutoMod · Swearing, level 2',
});

const msg = (id: string, at: number) =>
  ({ id, timestamp: String(at), tags: new Map([['tmi-sent-ts', String(at)]]) }) as unknown as BackendChatMessage;

const ids = (list: (string | BackendChatMessage)[]) => list.map((m) => (typeof m === 'string' ? m : m.id));

describe('held rows', () => {
  it('builds a row the list can key and recognize', () => {
    const row = heldRowMessage({ held: held('m9', 1000), line: null, by: null });
    expect(row.id).toBe('held-m9');
    expect(isHeldRowId(row.id)).toBe(true);
    expect(isHeldRowId('abc')).toBe(false);
    expect(sentAt(row)).toBe(1000);
    expect(row.content).toBe('this game is broken');
  });

  it('reads send times from objects and raw IRC lines', () => {
    expect(sentAt('@badges=;tmi-sent-ts=1234;id=x :a!a@a PRIVMSG #c :hi')).toBe(1234);
    expect(sentAt('@badges=;id=x :a!a@a PRIVMSG #c :hi')).toBeNull();
  });

  it('places each held row after the last message at or before it', () => {
    const list = [msg('a', 100), msg('b', 200), msg('c', 300)];
    const rows = [heldRowMessage({ held: held('h1', 250), line: null, by: null }), heldRowMessage({ held: held('h2', 400), line: null, by: null })];
    expect(ids(mergeHeld(list, rows))).toEqual(['a', 'b', 'held-h1', 'c', 'held-h2']);
  });

  it('leaves out rows older than everything shown, and appends when nothing has a time', () => {
    const old = heldRowMessage({ held: held('h0', 50), line: null, by: null });
    expect(ids(mergeHeld([msg('a', 100)], [old]))).toEqual(['a']);
    const untimed = [{ id: 'x', tags: new Map() } as unknown as BackendChatMessage];
    expect(ids(mergeHeld(untimed, [old]))).toEqual(['x', 'held-h0']);
  });

  it('keeps waiting and decided rows in time order with the folded line', () => {
    const rows = heldRows([held('h2', 300)], [{ held: held('h1', 100), status: 'denied', line: 'Denied Grumpy_Gus’s held message', moderator: 'kay' }]);
    expect(rows.map((r) => r.held.message_id)).toEqual(['h1', 'h2']);
    expect(rows[0].line).toBe('Denied Grumpy_Gus’s held message');
    expect(rows[0].by).toBe('kay');
    expect(rows[1].line).toBeNull();
  });
});
