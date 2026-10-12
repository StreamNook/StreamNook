// Run with: npm test
//
// The overlay builder's undo history (profileSync.ts, EditHistory): steps,
// one step per gesture, undo and redo, the redo branch a new edit drops, the
// cap, one history per overlay, and the keys that ask for each action.

import { describe, expect, it } from 'vitest';
import { EDIT_COALESCE_MS, EDIT_HISTORY_LIMIT, EditHistory, historyShortcut, type HistoryShortcutKey } from './profileSync';

const A = 'overlay-a';
const B = 'overlay-b';

// Apply a run of edits the way the builder does: each change records the
// value it replaced. Returns the final value.
function edit(h: EditHistory<number>, key: string, from: number, to: number[], at: number[], holding = false): number {
  let current = from;
  to.forEach((v, i) => {
    h.record(key, current, at[i], holding);
    current = v;
  });
  return current;
}

describe('EditHistory', () => {
  it('starts empty', () => {
    const h = new EditHistory<number>();
    expect(h.canUndo(A)).toBe(false);
    expect(h.canRedo(A)).toBe(false);
    expect(h.undo(A, 1)).toBeNull();
    expect(h.redo(A, 1)).toBeNull();
  });

  it('records separate edits as separate steps and undoes them in order', () => {
    const h = new EditHistory<number>();
    let cur = edit(h, A, 0, [1], [0]);
    h.seal(A);
    cur = edit(h, A, cur, [2], [10_000]);
    h.seal(A);
    expect(cur).toBe(2);
    expect(h.undo(A, cur)).toBe(1);
    expect(h.undo(A, 1)).toBe(0);
    expect(h.undo(A, 0)).toBeNull();
  });

  it('coalesces changes that land within the window into one step', () => {
    const h = new EditHistory<number>();
    const step = EDIT_COALESCE_MS - 50;
    const cur = edit(h, A, 0, [1, 2, 3, 4], [0, step, 2 * step, 3 * step]);
    expect(h.undo(A, cur)).toBe(0);
    expect(h.canUndo(A)).toBe(false);
  });

  it('starts a new step after a pause longer than the window', () => {
    const h = new EditHistory<number>();
    const cur = edit(h, A, 0, [1, 2], [0, EDIT_COALESCE_MS + 1]);
    expect(h.undo(A, cur)).toBe(1);
    expect(h.undo(A, 1)).toBe(0);
  });

  it('keeps a held drag as one step however long it pauses', () => {
    const h = new EditHistory<number>();
    const cur = edit(h, A, 10, [11, 12, 13], [0, 5_000, 20_000], true);
    h.seal(A); // pointer up
    expect(h.undo(A, cur)).toBe(10);
    expect(h.canUndo(A)).toBe(false);
  });

  it('a seal splits quick changes (two toggle clicks are two steps)', () => {
    const h = new EditHistory<number>();
    h.record(A, 0, 0);
    h.seal(A);
    h.record(A, 1, 50);
    h.seal(A);
    expect(h.undo(A, 2)).toBe(1);
    expect(h.undo(A, 1)).toBe(0);
  });

  it('redoes what was undone, a step at a time', () => {
    const h = new EditHistory<number>();
    h.record(A, 0, 0);
    h.seal(A);
    h.record(A, 1, 10_000);
    h.seal(A);
    expect(h.undo(A, 2)).toBe(1);
    expect(h.undo(A, 1)).toBe(0);
    expect(h.canRedo(A)).toBe(true);
    expect(h.redo(A, 0)).toBe(1);
    expect(h.redo(A, 1)).toBe(2);
    expect(h.redo(A, 2)).toBeNull();
    expect(h.undo(A, 2)).toBe(1);
  });

  it('a change after an undo never joins the step before it', () => {
    const h = new EditHistory<number>();
    h.record(A, 0, 0); // open step, current 1
    expect(h.undo(A, 1)).toBe(0);
    h.record(A, 0, 10); // well within the window
    expect(h.undo(A, 5)).toBe(0);
    expect(h.canUndo(A)).toBe(false);
  });

  it('a new edit after undoing drops the redo branch', () => {
    const h = new EditHistory<number>();
    h.record(A, 0, 0);
    h.seal(A);
    h.record(A, 1, 10_000);
    h.seal(A);
    expect(h.undo(A, 2)).toBe(1);
    h.record(A, 1, 20_000); // 1 -> 7
    expect(h.canRedo(A)).toBe(false);
    expect(h.redo(A, 7)).toBeNull();
    expect(h.undo(A, 7)).toBe(1);
    expect(h.undo(A, 1)).toBe(0);
  });

  it('keeps at most the limit, dropping the oldest steps', () => {
    const h = new EditHistory<number>();
    for (let i = 0; i < EDIT_HISTORY_LIMIT + 20; i++) {
      h.record(A, i, i * 1_000);
      h.seal(A);
    }
    let cur = EDIT_HISTORY_LIMIT + 20;
    let steps = 0;
    for (let v = h.undo(A, cur); v !== null; v = h.undo(A, cur)) {
      cur = v;
      steps++;
    }
    expect(steps).toBe(EDIT_HISTORY_LIMIT);
    expect(cur).toBe(20);
  });

  it('a custom limit and window apply', () => {
    const h = new EditHistory<number>(2, 100);
    h.record(A, 0, 0);
    h.record(A, 1, 150);
    h.record(A, 2, 300);
    expect(h.undo(A, 3)).toBe(2);
    expect(h.undo(A, 2)).toBe(1);
    expect(h.undo(A, 1)).toBeNull();
  });

  it('keeps one history per overlay', () => {
    const h = new EditHistory<number>();
    h.record(A, 0, 0);
    h.record(B, 100, 0);
    h.seal();
    h.record(B, 101, 10);
    expect(h.undo(A, 1)).toBe(0);
    expect(h.canUndo(A)).toBe(false);
    expect(h.undo(B, 102)).toBe(101);
    expect(h.undo(B, 101)).toBe(100);
    expect(h.canRedo(A)).toBe(true);
    expect(h.canRedo(B)).toBe(true);
  });

  it('sealing one overlay leaves the other open', () => {
    const h = new EditHistory<number>();
    h.record(A, 0, 0);
    h.record(B, 100, 0);
    h.seal(A);
    h.record(A, 1, 10);
    h.record(B, 101, 10);
    expect(h.undo(A, 2)).toBe(1);
    expect(h.undo(B, 102)).toBe(100);
  });

  it('forgetting an overlay drops only its history', () => {
    const h = new EditHistory<number>();
    h.record(A, 0, 0);
    h.record(B, 100, 0);
    h.forget(A);
    expect(h.canUndo(A)).toBe(false);
    expect(h.canUndo(B)).toBe(true);
    h.record(A, 5, 10);
    expect(h.undo(A, 6)).toBe(5);
  });
});

const key = (k: string, mods: Partial<HistoryShortcutKey> = {}): HistoryShortcutKey => ({
  key: k,
  code: '',
  ctrlKey: false,
  metaKey: false,
  shiftKey: false,
  altKey: false,
  ...mods,
});

describe('historyShortcut', () => {
  it('Ctrl+Z undoes; Ctrl+Shift+Z and Ctrl+Y redo', () => {
    expect(historyShortcut(key('z', { ctrlKey: true }), false)).toBe('undo');
    expect(historyShortcut(key('Z', { ctrlKey: true, shiftKey: true }), false)).toBe('redo');
    expect(historyShortcut(key('y', { ctrlKey: true }), false)).toBe('redo');
  });

  it('uses Cmd on macOS', () => {
    expect(historyShortcut(key('z', { metaKey: true }), true)).toBe('undo');
    expect(historyShortcut(key('z', { metaKey: true, shiftKey: true }), true)).toBe('redo');
    expect(historyShortcut(key('y', { metaKey: true }), true)).toBe('redo');
    expect(historyShortcut(key('z', { ctrlKey: true }), true)).toBeNull();
    expect(historyShortcut(key('z', { metaKey: true }), false)).toBeNull();
  });

  it('ignores other keys and modifier mixes', () => {
    expect(historyShortcut(key('z'), false)).toBeNull();
    expect(historyShortcut(key('z', { shiftKey: true }), false)).toBeNull();
    expect(historyShortcut(key('z', { ctrlKey: true, altKey: true }), false)).toBeNull();
    expect(historyShortcut(key('z', { ctrlKey: true, metaKey: true }), false)).toBeNull();
    expect(historyShortcut(key('y', { ctrlKey: true, shiftKey: true }), false)).toBeNull();
    expect(historyShortcut(key('x', { ctrlKey: true }), false)).toBeNull();
  });

  it('reads the physical key when the layout types another script', () => {
    expect(historyShortcut(key('я', { code: 'KeyZ', ctrlKey: true }), false)).toBe('undo');
    expect(historyShortcut(key('н', { code: 'KeyY', ctrlKey: true }), false)).toBe('redo');
  });

  it('follows the letter, not the position, on Latin layouts (QWERTZ)', () => {
    // The key labelled Z sits where QWERTY has Y.
    expect(historyShortcut(key('z', { code: 'KeyY', ctrlKey: true }), false)).toBe('undo');
  });
});
