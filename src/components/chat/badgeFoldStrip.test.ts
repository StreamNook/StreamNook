import { describe, expect, it, vi } from 'vitest';
import { activeStripToken, chipLeftView, openStrip, releaseStrip, stripPlace, type Box } from './badgeFoldStrip';

const box = (left: number, top: number, right: number, bottom: number): Box => ({ left, top, right, bottom });

describe('the one open strip', () => {
  it('opening a second closes the first once, and release only clears the owner', () => {
    const a = { token: {}, close: vi.fn() };
    const b = { token: {}, close: vi.fn() };
    openStrip(a);
    openStrip(b);
    expect(a.close).toHaveBeenCalledTimes(1);
    expect(b.close).not.toHaveBeenCalled();
    expect(activeStripToken()).toBe(b.token);
    releaseStrip(a.token);
    expect(activeStripToken()).toBe(b.token);
    releaseStrip(b.token);
    expect(activeStripToken()).toBeNull();
  });
});

describe('chipLeftView', () => {
  const view = box(0, 100, 400, 500);
  it('is true for a chip above, below, left or right of the view', () => {
    expect(chipLeftView(box(10, 60, 40, 100), view)).toBe(true);
    expect(chipLeftView(box(10, 500, 40, 520), view)).toBe(true);
    expect(chipLeftView(box(-50, 200, 0, 220), view)).toBe(true);
    expect(chipLeftView(box(400, 200, 430, 220), view)).toBe(true);
  });
  it('is false inside the view', () => {
    expect(chipLeftView(box(10, 200, 40, 220), view)).toBe(false);
  });
});

describe('stripPlace', () => {
  it('opens rightward when the strip fits on the right', () => {
    const chip = box(100, 10, 120, 30);
    const p = stripPlace(chip, 1000, 120);
    expect(p.dir).toBe(1);
    expect(p.left).toBe(chip.right + 2);
    expect(p.right).toBeNull();
  });
  it('opens leftward near the right edge', () => {
    const chip = box(900, 10, 920, 30);
    const p = stripPlace(chip, 1000, 120);
    expect(p.dir).toBe(-1);
    expect(p.left).toBeNull();
    expect(p.right).toBe(1000 - 900 + 2);
    expect(p.maxWidth).toBe(890);
  });
  it('picks the roomier side when neither fits', () => {
    expect(stripPlace(box(400, 0, 420, 20), 700, 500).dir).toBe(-1);
    expect(stripPlace(box(100, 0, 120, 20), 700, 900).dir).toBe(1);
  });
  it('lockDir keeps the chosen side even where the other would fit', () => {
    const p = stripPlace(box(100, 10, 120, 30), 1000, 120, -1);
    expect(p.dir).toBe(-1);
    expect(p.right).toBe(1000 - 100 + 2);
  });
  it('never has a max width under 40 and never overlaps the chip', () => {
    for (const chip of [box(0, 0, 20, 20), box(980, 0, 1000, 20), box(500, 0, 520, 20)]) {
      for (const lock of [undefined, 1, -1] as const) {
        const p = stripPlace(chip, 1000, 200, lock);
        expect(p.maxWidth).toBeGreaterThanOrEqual(40);
        if (p.left !== null) expect(p.left).toBeGreaterThanOrEqual(chip.right);
        if (p.right !== null) expect(1000 - p.right).toBeLessThanOrEqual(chip.left);
      }
    }
  });
});
