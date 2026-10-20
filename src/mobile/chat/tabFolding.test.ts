import { describe, expect, it } from 'vitest';
import { pickFolding, rowWidth, TAB_GAP, type FoldTab } from './tabFolding';

const tab = (channel: string, label: string, full: number): FoldTab => ({
  channel,
  label,
  full,
  uncapped: full + 30,
  folded: 32,
  foldedTight: 20,
});

const rooms = [tab('a', 'aydan', 100), tab('s', 'summit1g', 120), tab('x', 'xQc', 80)];

describe('room strip folding (the iPhone strip rule)', () => {
  it('folds nothing while the rooms fit', () => {
    const fit = rowWidth(rooms, 's', new Set(), false);
    expect(fit).toBe(300 + 2 * TAB_GAP);
    expect(pickFolding(rooms, 's', fit)).toEqual({ folded: new Set(), tight: false });
  });

  it('folds rooms behind the one on screen, longest name first, one at a time', () => {
    // One short of fitting: "aydan" (the longest name not on screen) folds first.
    const one = pickFolding(rooms, 's', 300 + 2 * TAB_GAP - 1);
    expect(one).toEqual({ folded: new Set(['a']), tight: false });
    // Tighter still: "xQc" folds too, and the room on screen is never folded.
    const both = pickFolding(rooms, 's', 32 + 150 + 32 + 2 * TAB_GAP);
    expect(both).toEqual({ folded: new Set(['a', 'x']), tight: false });
  });

  it('the room on screen takes the width the others gave up', () => {
    // Folded others: the active room is measured uncapped (its whole name).
    expect(rowWidth(rooms, 's', new Set(['a']), false)).toBe(32 + 150 + 80 + 2 * TAB_GAP);
  });

  it('past every fold, the tightest row, where the name on screen gives way', () => {
    expect(pickFolding(rooms, 's', 100)).toEqual({ folded: new Set(['a', 'x']), tight: true });
  });

  it('one room never folds', () => {
    expect(pickFolding([tab('s', 'summit1g', 120)], 's', 10)).toEqual({ folded: new Set(), tight: true });
  });
});
