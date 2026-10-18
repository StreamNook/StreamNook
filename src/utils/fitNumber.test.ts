import { describe, expect, it } from 'vitest';
import { FIT_AMOUNT_MAX_CHARS, FIT_MAX_CHARS, fitAmount, fitCount, fitSeconds } from './fitNumber';

describe('fitSeconds', () => {
  it('shows seconds while they fit', () => {
    expect(fitSeconds(5)).toBe('5');
    expect(fitSeconds(30)).toBe('30');
    expect(fitSeconds(120)).toBe('120');
    expect(fitSeconds(999)).toBe('999');
  });

  it('rounds a part second up', () => {
    expect(fitSeconds(0.2)).toBe('1');
    expect(fitSeconds(29.01)).toBe('30');
  });

  it('moves to minutes, hours, then days', () => {
    expect(fitSeconds(1000)).toBe('17m');
    expect(fitSeconds(99 * 60)).toBe('99m');
    expect(fitSeconds(99 * 60 + 1)).toBe('2h');
    expect(fitSeconds(99 * 3600)).toBe('99h');
    expect(fitSeconds(99 * 3600 + 1)).toBe('5d');
  });

  it('reads 0 for nothing usable', () => {
    expect(fitSeconds(0)).toBe('0');
    expect(fitSeconds(-4)).toBe('0');
    expect(fitSeconds(Number.NaN)).toBe('0');
    expect(fitSeconds(Number.POSITIVE_INFINITY)).toBe('0');
  });

  it('never goes past three characters, even for the epoch in seconds', () => {
    // The slow-mode bug put Unix seconds on the send button.
    for (const s of [1, 59, 60, 999, 1000, 5999, 6000, 359_999, 360_000, 1_792_251_800, Number.MAX_SAFE_INTEGER]) {
      expect(fitSeconds(s).length).toBeLessThanOrEqual(FIT_MAX_CHARS);
    }
  });
});

describe('fitCount', () => {
  it('shows the count up to 99, then 99+', () => {
    expect(fitCount(1)).toBe('1');
    expect(fitCount(99)).toBe('99');
    expect(fitCount(100)).toBe('99+');
    expect(fitCount(1_792_251_800)).toBe('99+');
  });

  it('reads 0 for nothing usable', () => {
    expect(fitCount(0)).toBe('0');
    expect(fitCount(-1)).toBe('0');
    expect(fitCount(Number.NaN)).toBe('0');
  });

  it('takes a lower cap for a smaller slot', () => {
    expect(fitCount(9, 9)).toBe('9');
    expect(fitCount(10, 9)).toBe('9+');
    expect(fitCount(1_792_251_800, 9)).toBe('9+');
  });
});

describe('fitAmount', () => {
  it('keeps the scale, rounded down', () => {
    expect(fitAmount(640)).toBe('640');
    expect(fitAmount(999)).toBe('999');
    expect(fitAmount(1000)).toBe('1K');
    expect(fitAmount(2134)).toBe('2K');
    expect(fitAmount(45_999)).toBe('45K');
    expect(fitAmount(999_999)).toBe('999K');
    expect(fitAmount(1_000_000)).toBe('1M');
    expect(fitAmount(12_345_678)).toBe('12M');
    expect(fitAmount(1_792_251_800)).toBe('1B');
  });

  it('reads 0 for nothing usable', () => {
    expect(fitAmount(0)).toBe('0');
    expect(fitAmount(-3)).toBe('0');
    expect(fitAmount(Number.NaN)).toBe('0');
  });

  it('never goes past four characters', () => {
    for (const n of [1, 999, 1000, 99_999, 999_999, 1e6, 999_999_999, 1e12, Number.MAX_SAFE_INTEGER]) {
      expect(fitAmount(n).length).toBeLessThanOrEqual(FIT_AMOUNT_MAX_CHARS);
    }
  });
});
