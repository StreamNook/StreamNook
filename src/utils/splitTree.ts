// Binary split tree for MultiChat panes (Chatterino-style nested splits).
//
// A leaf shows one channel (`key` = composite provider:channel entry key) or
// follows the active tab (`key === null`). A branch divides its area between
// two children along `dir` at `ratio` (fraction given to `a`). Pure data,
// pure functions: the window persists it as JSON and re-renders on change.

export type SplitDir = 'row' | 'col';

export interface SplitLeaf {
  kind: 'leaf';
  id: string;
  /** Entry key of the channel shown, or null to follow the active tab. */
  key: string | null;
}

export interface SplitBranch {
  kind: 'split';
  id: string;
  dir: SplitDir;
  /** Fraction of the axis given to `a`, clamped to [MIN_RATIO, 1 - MIN_RATIO]. */
  ratio: number;
  a: SplitNode;
  b: SplitNode;
}

export type SplitNode = SplitLeaf | SplitBranch;

export const MIN_RATIO = 0.12;

let counter = 0;
export function newSplitId(): string {
  counter += 1;
  const rand =
    typeof crypto !== 'undefined' && 'randomUUID' in crypto
      ? crypto.randomUUID().slice(0, 8)
      : Math.random().toString(36).slice(2, 10);
  return `${rand}-${counter}`;
}

export function leaf(key: string | null): SplitLeaf {
  return { kind: 'leaf', id: newSplitId(), key };
}

export function clampRatio(r: number): number {
  if (!Number.isFinite(r)) return 0.5;
  return Math.min(1 - MIN_RATIO, Math.max(MIN_RATIO, r));
}

/** Equal-width row of leaves, nested right-heavy so every divider is draggable. */
export function fromColumns(keys: Array<string | null>): SplitNode {
  if (keys.length === 0) return leaf(null);
  const build = (i: number): SplitNode => {
    if (i === keys.length - 1) return leaf(keys[i]);
    const remaining = keys.length - i;
    return {
      kind: 'split',
      id: newSplitId(),
      dir: 'row',
      ratio: clampRatio(1 / remaining),
      a: leaf(keys[i]),
      b: build(i + 1),
    };
  };
  return build(0);
}

export function leaves(tree: SplitNode): SplitLeaf[] {
  if (tree.kind === 'leaf') return [tree];
  return [...leaves(tree.a), ...leaves(tree.b)];
}

export function leafCount(tree: SplitNode): number {
  return tree.kind === 'leaf' ? 1 : leafCount(tree.a) + leafCount(tree.b);
}

/** Width of the layout in leaves: rows add, columns take the wider child. */
export function columnCount(tree: SplitNode): number {
  if (tree.kind === 'leaf') return 1;
  return tree.dir === 'row'
    ? columnCount(tree.a) + columnCount(tree.b)
    : Math.max(columnCount(tree.a), columnCount(tree.b));
}

export function findLeaf(tree: SplitNode, id: string): SplitLeaf | null {
  if (tree.kind === 'leaf') return tree.id === id ? tree : null;
  return findLeaf(tree.a, id) ?? findLeaf(tree.b, id);
}

function map(tree: SplitNode, fn: (n: SplitNode) => SplitNode): SplitNode {
  const mapped = fn(tree);
  if (mapped !== tree) return mapped;
  if (tree.kind === 'leaf') return tree;
  const a = map(tree.a, fn);
  const b = map(tree.b, fn);
  return a === tree.a && b === tree.b ? tree : { ...tree, a, b };
}

/** Replace `leafId` with a split of it and a new leaf showing `newKey`. */
export function splitLeaf(tree: SplitNode, leafId: string, dir: SplitDir, newKey: string | null): SplitNode {
  return map(tree, (n) =>
    n.kind === 'leaf' && n.id === leafId
      ? { kind: 'split', id: newSplitId(), dir, ratio: 0.5, a: n, b: leaf(newKey) }
      : n,
  );
}

/** Remove a leaf; its sibling takes the parent's place. Null when it was the last leaf. */
export function closeLeaf(tree: SplitNode, leafId: string): SplitNode | null {
  if (tree.kind === 'leaf') return tree.id === leafId ? null : tree;
  if (tree.a.kind === 'leaf' && tree.a.id === leafId) return tree.b;
  if (tree.b.kind === 'leaf' && tree.b.id === leafId) return tree.a;
  const a = closeLeaf(tree.a, leafId);
  const b = closeLeaf(tree.b, leafId);
  if (a === null) return b;
  if (b === null) return a;
  return a === tree.a && b === tree.b ? tree : { ...tree, a, b };
}

export function setRatio(tree: SplitNode, splitId: string, ratio: number): SplitNode {
  return map(tree, (n) =>
    n.kind === 'split' && n.id === splitId ? { ...n, ratio: clampRatio(ratio) } : n,
  );
}

export function setLeafKey(tree: SplitNode, leafId: string, key: string | null): SplitNode {
  return map(tree, (n) => (n.kind === 'leaf' && n.id === leafId ? { ...n, key } : n));
}

/** A channel left the window: close its leaves, or blank the last one. */
export function removeKey(tree: SplitNode, key: string): SplitNode {
  let next: SplitNode | null = tree;
  for (const l of leaves(tree)) {
    if (l.key !== key || next === null) continue;
    const closed = closeLeaf(next, l.id);
    if (closed === null) {
      // Last leaf: keep the layout, follow the active tab instead.
      return setLeafKey(next, l.id, null);
    }
    next = closed;
  }
  return next ?? tree;
}

/** True when any leaf shows `key`, or a null leaf would (when key is active). */
export function showsKey(tree: SplitNode, key: string, activeKey: string | null): boolean {
  return leaves(tree).some((l) => l.key === key || (l.key === null && activeKey === key));
}

/** Structural check for persisted JSON. */
export function isSplitNode(v: unknown, depth = 0): v is SplitNode {
  if (!v || typeof v !== 'object' || depth > 16) return false;
  const n = v as Record<string, unknown>;
  if (typeof n.id !== 'string') return false;
  if (n.kind === 'leaf') return n.key === null || typeof n.key === 'string';
  if (n.kind === 'split') {
    return (
      (n.dir === 'row' || n.dir === 'col') &&
      typeof n.ratio === 'number' &&
      isSplitNode(n.a, depth + 1) &&
      isSplitNode(n.b, depth + 1)
    );
  }
  return false;
}
