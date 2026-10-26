import { describe, expect, it } from 'vitest';
import { patchHeldRows } from './heldRowPatch';

describe('patchHeldRows', () => {
  it('swaps named rows in place and leaves the rest of the queue alone', () => {
    const held: unknown[] = [
      { id: 'a', segments: ['dance'] },
      'raw irc line',
      { id: 'b', segments: ['hi'] },
    ];
    const byId = new Map([['a', { segments: ['EMOTE'] }]]);
    patchHeldRows(held, byId, (m, u) => ({ ...m, ...u }));
    expect(held).toEqual([{ id: 'a', segments: ['EMOTE'] }, 'raw irc line', { id: 'b', segments: ['hi'] }]);
    expect(byId.size).toBe(0);
  });

  it('keeps the ids it could not place, for the next place to look', () => {
    const held: unknown[] = [{ id: 'a' }];
    const byId = new Map([
      ['a', 1],
      ['later', 2],
    ]);
    patchHeldRows(held, byId, (m) => m);
    expect([...byId.keys()]).toEqual(['later']);
  });
});
