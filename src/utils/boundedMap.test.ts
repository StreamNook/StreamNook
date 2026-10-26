import { describe, expect, it } from 'vitest';

import { boundedAdd, boundedSet } from './boundedMap';

describe('boundedSet', () => {
  it('holds the cap and drops the oldest-inserted key first', () => {
    const map = new Map<string, number>();
    for (let i = 0; i <= 5; i++) boundedSet(map, `k${i}`, i, 5);
    expect(map.size).toBe(5);
    expect(map.has('k0')).toBe(false);
    expect([...map.keys()]).toEqual(['k1', 'k2', 'k3', 'k4', 'k5']);
  });

  it('rewriting a key at the cap evicts nothing', () => {
    const map = new Map<string, number>();
    for (let i = 0; i < 3; i++) boundedSet(map, `k${i}`, i, 3);
    boundedSet(map, 'k1', 99, 3);
    expect(map.size).toBe(3);
    expect(map.get('k1')).toBe(99);
    expect(map.has('k0')).toBe(true);
  });
});

describe('boundedAdd', () => {
  it('holds the cap and drops the oldest-inserted value first', () => {
    const set = new Set<string>();
    for (let i = 0; i <= 5; i++) boundedAdd(set, `v${i}`, 5);
    expect(set.size).toBe(5);
    expect(set.has('v0')).toBe(false);
    expect([...set]).toEqual(['v1', 'v2', 'v3', 'v4', 'v5']);
  });

  it('adding a value already present at the cap evicts nothing', () => {
    const set = new Set<string>();
    for (let i = 0; i < 3; i++) boundedAdd(set, `v${i}`, 3);
    boundedAdd(set, 'v1', 3);
    expect(set.size).toBe(3);
    expect(set.has('v0')).toBe(true);
  });
});
