import { describe, expect, it } from 'vitest';
import { badgeLinkOutcome, tileForKey } from './badgeLinkTile';

const tiles = [
  { title: 'A', keys: ['clover/1'] },
  { title: 'B', keys: ['payday-3/1', 'Payday-3/2'] },
];

describe('tileForKey', () => {
  it('matches a tile by any of its keys', () => {
    expect(tileForKey(tiles, 'clover/1')?.title).toBe('A');
    expect(tileForKey(tiles, 'payday-3/1')?.title).toBe('B');
    expect(tileForKey(tiles, 'payday-3/2')?.title).toBe('B');
  });

  it('compares the set id without case', () => {
    expect(tileForKey(tiles, 'CLOVER/1')?.title).toBe('A');
    expect(tileForKey(tiles, 'payday-3/2')?.title).toBe('B');
  });

  it('compares the version exactly', () => {
    expect(tileForKey(tiles, 'clover/2')).toBeUndefined();
    expect(tileForKey(tiles, 'clover/10')).toBeUndefined();
  });

  it('finds nothing for an unknown set', () => {
    expect(tileForKey(tiles, 'nope/1')).toBeUndefined();
  });
});

describe('badgeLinkOutcome', () => {
  it('does nothing while the catalogue is not there', () => {
    expect(badgeLinkOutcome({ loading: true, tiles }, 'clover/1')).toEqual({ kind: 'none' });
  });

  it('opens the tile', () => {
    expect(badgeLinkOutcome({ loading: false, tiles }, 'clover/1')).toEqual({ kind: 'open', tile: tiles[0] });
  });

  it('reports a miss', () => {
    expect(badgeLinkOutcome({ loading: false, tiles }, 'nope/1')).toEqual({ kind: 'miss' });
  });
});
