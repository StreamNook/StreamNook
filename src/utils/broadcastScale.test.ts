import { describe, expect, it } from 'vitest';
import { broadcastScale, formatBehindShort, SECTION_FACTORS } from './broadcastScale';

describe('broadcastScale', () => {
  const total = 4 * 3600; // a four-hour broadcast
  const scale = broadcastScale(total);

  it('pins the ends', () => {
    expect(scale.toFrac(total)).toBe(1);
    expect(scale.toFrac(0)).toBe(0);
    expect(scale.toSecs(1)).toBe(total);
    expect(scale.toSecs(0)).toBe(0);
  });

  it('gives the last stretch before live a whole section at 1x', () => {
    const base = total / SECTION_FACTORS.reduce((a, b) => a + b, 0);
    // The boundary between the 1x and 2x sections is one base unit behind live.
    expect(scale.toFrac(total - base)).toBeCloseTo(0.8, 6);
    // Halfway into the 1x section is half a base unit behind live.
    expect(scale.toSecs(0.9)).toBeCloseTo(total - base / 2, 6);
  });

  it('compresses each section to the left by another factor of two', () => {
    const base = total / 31;
    expect(scale.toSecs(0.8)).toBeCloseTo(total - base, 6); // 1x/2x boundary
    expect(scale.toSecs(0.6)).toBeCloseTo(total - 3 * base, 6); // 2x/4x
    expect(scale.toSecs(0.4)).toBeCloseTo(total - 7 * base, 6); // 4x/8x
    expect(scale.toSecs(0.2)).toBeCloseTo(total - 15 * base, 6); // 8x/16x
  });

  it('round-trips everywhere', () => {
    for (let f = 0; f <= 1.0001; f += 0.05) {
      expect(scale.toFrac(scale.toSecs(f))).toBeCloseTo(Math.min(1, f), 9);
    }
    for (let s = 0; s <= total; s += 977) {
      expect(scale.toSecs(scale.toFrac(s))).toBeCloseTo(s, 6);
    }
  });

  it('lists the four inner boundaries left to right', () => {
    expect(scale.ticks.map((t) => +t.frac.toFixed(2))).toEqual([0.2, 0.4, 0.6, 0.8]);
    const base = total / 31;
    expect(scale.ticks.map((t) => Math.round(t.behindSecs / base))).toEqual([15, 7, 3, 1]);
  });

  it('survives a degenerate total', () => {
    const s = broadcastScale(0);
    expect(s.toSecs(0.5)).toBeGreaterThanOrEqual(0);
    expect(Number.isFinite(s.toFrac(0.5))).toBe(true);
  });
});

describe('formatBehindShort', () => {
  it('formats boundaries compactly', () => {
    expect(formatBehindShort(465)).toBe('8m');
    expect(formatBehindShort(3720)).toBe('1h 02m');
    expect(formatBehindShort(7200)).toBe('2h');
    expect(formatBehindShort(20)).toBe('20s');
  });
});
