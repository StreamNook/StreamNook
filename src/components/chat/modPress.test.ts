import { describe, expect, it } from 'vitest';

import { edgesMoved, isStill, sameBox, STILL_MS, tapAgain, TAP_AGAIN_MS, TAP_AGAIN_PX, type Box, type Tap } from './modPress';

const box = (top: number, left = 10, h = 20, w = 30): Box => ({ top, left, bottom: top + h, right: left + w });
const tap = (x: number, y: number, t: number, b: Box | null): Tap => ({ x, y, t, box: b });

describe('edgesMoved', () => {
  it('is a slide when both edges moved', () => {
    expect(edgesMoved({ top: 100, bottom: 130 }, { top: 70, bottom: 100 })).toBe(true);
  });

  it('is no slide when the row grew downward in place', () => {
    expect(edgesMoved({ top: 100, bottom: 130 }, { top: 100, bottom: 151 })).toBe(false);
  });

  it('is no slide when the row grew upward at a pinned bottom', () => {
    expect(edgesMoved({ top: 100, bottom: 130 }, { top: 79, bottom: 130 })).toBe(false);
  });

  it('ignores half a pixel or less', () => {
    expect(edgesMoved({ top: 100, bottom: 130 }, { top: 100.5, bottom: 130.5 })).toBe(false);
    expect(edgesMoved({ top: 100, bottom: 130 }, { top: 100.6, bottom: 130.6 })).toBe(true);
  });
});

describe('isStill', () => {
  it('is still when nothing ever moved', () => {
    expect(isStill(null, 5)).toBe(true);
  });

  it('waits STILL_MS after the last move', () => {
    expect(isStill(1000, 1000 + STILL_MS - 1)).toBe(false);
    expect(isStill(1000, 1000 + STILL_MS)).toBe(true);
  });
});

describe('sameBox', () => {
  it('matches a button re-rendered in place', () => {
    expect(sameBox(box(50), { ...box(50), top: 50.4 })).toBe(true);
  });

  it('tells apart buttons in other places', () => {
    expect(sameBox(box(50), box(80))).toBe(false);
    expect(sameBox(box(50, 10), box(50, 60))).toBe(false);
  });

  it('matches no box only with no box', () => {
    expect(sameBox(null, null)).toBe(true);
    expect(sameBox(null, box(50))).toBe(false);
  });
});

describe('tapAgain', () => {
  const first = tap(100, 100, 1000, null);

  it('catches a quick second tap on a button the first tap brought up', () => {
    expect(tapAgain(tap(102, 101, 1200, box(90)), first)).toBe(true);
  });

  it('lets a second tap on the same button through (Ban, press again)', () => {
    const onBan = tap(100, 100, 1000, box(90));
    expect(tapAgain(tap(101, 100, 1200, { ...box(90) }), onBan)).toBe(false);
  });

  it('lets a tap through once the window has passed', () => {
    expect(tapAgain(tap(100, 100, 1000 + TAP_AGAIN_MS, box(90)), first)).toBe(false);
  });

  it('lets a tap elsewhere through', () => {
    expect(tapAgain(tap(100 + TAP_AGAIN_PX + 1, 100, 1100, box(90)), first)).toBe(false);
  });

  it('never blocks a tap on no button, or the first tap', () => {
    expect(tapAgain(tap(100, 100, 1100, null), first)).toBe(false);
    expect(tapAgain(tap(100, 100, 1100, box(90)), null)).toBe(false);
  });
});
