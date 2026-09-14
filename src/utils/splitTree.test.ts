import { describe, expect, it } from 'vitest';
import {
  closeLeaf,
  columnCount,
  fromColumns,
  isSplitNode,
  leafCount,
  leaves,
  removeKey,
  setLeafKey,
  setRatio,
  showsKey,
  splitLeaf,
  MIN_RATIO,
} from './splitTree';

describe('splitTree', () => {
  it('builds an equal row of leaves', () => {
    const t = fromColumns(['a', 'b', 'c']);
    expect(leafCount(t)).toBe(3);
    expect(columnCount(t)).toBe(3);
    expect(leaves(t).map((l) => l.key)).toEqual(['a', 'b', 'c']);
    expect(t.kind === 'split' && Math.abs(t.ratio - 1 / 3) < 1e-9).toBe(true);
  });

  it('splits a leaf right and down and counts columns', () => {
    const t0 = fromColumns(['a']);
    const id = leaves(t0)[0].id;
    const t1 = splitLeaf(t0, id, 'row', 'b');
    expect(columnCount(t1)).toBe(2);
    const bId = leaves(t1)[1].id;
    const t2 = splitLeaf(t1, bId, 'col', null);
    expect(leafCount(t2)).toBe(3);
    expect(columnCount(t2)).toBe(2);
    expect(leaves(t2).map((l) => l.key)).toEqual(['a', 'b', null]);
  });

  it('closes a leaf and collapses the parent', () => {
    const t = fromColumns(['a', 'b', 'c']);
    const [la, lb] = leaves(t);
    const t1 = closeLeaf(t, lb.id);
    expect(t1 && leaves(t1).map((l) => l.key)).toEqual(['a', 'c']);
    const t2 = closeLeaf(t1!, la.id);
    expect(t2 && t2.kind).toBe('leaf');
    expect(closeLeaf(t2!, leaves(t2!)[0].id)).toBeNull();
  });

  it('clamps ratios and sets leaf keys', () => {
    const t = fromColumns(['a', 'b']);
    expect(t.kind).toBe('split');
    const t1 = setRatio(t, t.id, 0.01);
    expect(t1.kind === 'split' && t1.ratio).toBe(MIN_RATIO);
    const t2 = setLeafKey(t1, leaves(t1)[0].id, 'z');
    expect(leaves(t2)[0].key).toBe('z');
  });

  it('removes a key from every leaf but keeps the last one following tabs', () => {
    const t = fromColumns(['a', 'a', 'b']);
    const t1 = removeKey(t, 'a');
    expect(leaves(t1).map((l) => l.key)).toEqual(['b']);
    const t2 = removeKey(t1, 'b');
    expect(leaves(t2).map((l) => l.key)).toEqual([null]);
    expect(showsKey(t2, 'q', 'q')).toBe(true);
    expect(showsKey(t2, 'q', 'r')).toBe(false);
  });

  it('validates persisted shapes', () => {
    expect(isSplitNode(fromColumns(['a', null]))).toBe(true);
    expect(isSplitNode({ kind: 'leaf', id: 1, key: null })).toBe(false);
    expect(isSplitNode({ kind: 'split', id: 'x', dir: 'diag', ratio: 0.5, a: {}, b: {} })).toBe(false);
    expect(isSplitNode(null)).toBe(false);
  });
});
