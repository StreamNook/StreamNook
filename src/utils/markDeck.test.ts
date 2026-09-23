import { describe, expect, it } from 'vitest';
import { deckGeometry } from './markDeck';

// The two slots the deck is actually drawn in.
const FLYOUT = 12;
const ANCHOR = 16;

describe('deckGeometry', () => {
  // The guarantee that makes this change safe to land: with the three
  // platforms that shipped before TikTok, every number is what it was.
  it('reproduces the original steps at three marks', () => {
    const f = deckGeometry(FLYOUT, 3);
    expect([f.card, f.glyph, f.dx, f.dy]).toEqual([11, 8, 5, 3]);

    const a = deckGeometry(ANCHOR, 3);
    expect([a.card, a.glyph, a.dx, a.dy]).toEqual([14, 10, 6, 4]);
  });

  // The bug this replaced: span was card + dx*(n-1) with a FIXED dx, so a
  // fourth mark pushed the deck 26px wide in a 12px slot, overhanging by 7px
  // each side and colliding with the label 10px away.
  it('does not grow the deck when a fourth platform is added', () => {
    const three = deckGeometry(FLYOUT, 3);
    const four = deckGeometry(FLYOUT, 4);
    expect(four.spread).toBeLessThanOrEqual(three.spread);
    expect(four.height).toBeLessThanOrEqual(three.height);
  });

  it('keeps the overhang inside the gap beside the slot', () => {
    // 10px gap to the label, so at most 5px may hang off each side.
    for (const n of [2, 3, 4, 5, 6]) {
      const g = deckGeometry(FLYOUT, n);
      const overhang = (g.spread - FLYOUT) / 2;
      expect(overhang).toBeLessThanOrEqual(5);
    }
  });

  it('keeps the anchor deck inside the pill', () => {
    // The selector pill is 34px tall.
    for (const n of [2, 3, 4, 5, 6]) {
      expect(deckGeometry(ANCHOR, n).height).toBeLessThanOrEqual(34);
    }
  });

  it('degenerates safely at one mark', () => {
    const g = deckGeometry(FLYOUT, 1);
    expect([g.dx, g.dy]).toEqual([0, 0]);
    expect(g.spread).toBe(g.card);
    expect(g.height).toBe(g.card);
  });
});
