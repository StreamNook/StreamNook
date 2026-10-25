import { describe, expect, it } from 'vitest';
import { withClearedEntry } from './clearedContexts';

const e = (...ids: string[]) => ({ affectedMessageIds: new Set(ids) });

describe('withClearedEntry', () => {
  it('keeps marks with a row still present, drops the rest, adds the new one', () => {
    const before = new Map([
      ['old', e('m1')],
      ['visible', e('m2', 'm3')],
    ]);
    const next = withClearedEntry(before, new Set(['m3', 'm9']), 'new', e('m9'));
    expect([...next.keys()].sort()).toEqual(['new', 'visible']);
    expect(next).not.toBe(before);
  });

  it('a second action on the same user replaces its mark', () => {
    const next = withClearedEntry(new Map([['u', e('a')]]), new Set(['a', 'b']), 'u', e('b'));
    expect([...next.get('u')!.affectedMessageIds]).toEqual(['b']);
  });
});
