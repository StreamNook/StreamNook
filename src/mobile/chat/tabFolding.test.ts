import { describe, expect, it } from 'vitest';
import { pickFolding, rowWidth, TAB_GAP, TAB_GAP_TIGHT, type FoldTab } from './tabFolding';

const tab = (channel: string, label: string, full: number): FoldTab => ({
  channel,
  label,
  full,
  uncapped: full + 30,
  // No broadcast mark, the name at most NAME_STUB.
  stub: 90,
  folded: 32,
  foldedTight: 20,
});

const rooms = [tab('a', 'aydan', 100), tab('s', 'summit1g', 120), tab('x', 'xQc', 80)];

describe('room strip folding (the iPhone strip rule)', () => {
  it('folds nothing while the rooms fit', () => {
    const fit = rowWidth(rooms, 's', new Set(), false);
    expect(fit).toBe(300 + 2 * TAB_GAP);
    expect(pickFolding(rooms, 's', fit)).toEqual({ folded: new Set(), tight: false, bare: false });
  });

  it('folds rooms behind the one on screen, longest name first, one at a time', () => {
    // One short of fitting: "aydan" (the longest name not on screen) folds first.
    const one = pickFolding(rooms, 's', 300 + 2 * TAB_GAP - 1);
    expect(one).toEqual({ folded: new Set(['a']), tight: false, bare: false });
    // Tighter still: "xQc" folds too, and the room on screen is never folded.
    const both = pickFolding(rooms, 's', 32 + 150 + 32 + 2 * TAB_GAP);
    expect(both).toEqual({ folded: new Set(['a', 'x']), tight: false, bare: false });
  });

  it('the room on screen takes the width the others gave up', () => {
    // Folded others: the active room is measured uncapped (its whole name).
    expect(rowWidth(rooms, 's', new Set(['a']), false)).toBe(32 + 150 + 80 + 2 * TAB_GAP);
  });

  it('past every fold, the tightest row keeps the whole name while it fits', () => {
    const tight = 20 + 150 + 20 + 2 * TAB_GAP_TIGHT;
    expect(pickFolding(rooms, 's', tight)).toEqual({ folded: new Set(['a', 'x']), tight: true, bare: false });
  });

  it('then the room on screen drops its mark and its name is cut, no shorter than the stub', () => {
    const stub = 20 + 90 + 20 + 2 * TAB_GAP_TIGHT;
    expect(pickFolding(rooms, 's', stub)).toEqual({ folded: new Set(['a', 'x']), tight: true, bare: true });
  });

  it('narrower than the stub, every room is a face, the one on screen included', () => {
    expect(pickFolding(rooms, 's', 100)).toEqual({ folded: new Set(['a', 's', 'x']), tight: true, bare: true });
  });

  it('one room in a narrow side chat: its whole name, its name cut to the stub, or its face', () => {
    const one = [tab('s', 'summit1g', 120)];
    expect(pickFolding(one, 's', 120)).toEqual({ folded: new Set(), tight: false, bare: false });
    expect(pickFolding(one, 's', 95)).toEqual({ folded: new Set(), tight: true, bare: true });
    expect(pickFolding(one, 's', 60)).toEqual({ folded: new Set(['s']), tight: true, bare: true });
  });
});
