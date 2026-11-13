import { describe, expect, it } from 'vitest';
import { disclosureKey, progressPercent, readDisclosureOpen, writeDisclosureOpen } from './settingsKitPure';

describe('progressPercent', () => {
  it('is the share of the total, as a percent', () => {
    expect(progressPercent(7, 15)).toBeCloseTo(46.666, 2);
    expect(progressPercent(0, 15)).toBe(0);
    expect(progressPercent(15, 15)).toBe(100);
  });

  it('never leaves 0 to 100', () => {
    expect(progressPercent(20, 15)).toBe(100);
    expect(progressPercent(-3, 15)).toBe(0);
  });

  it('reads a missing or zero total as empty, not NaN', () => {
    expect(progressPercent(5, 0)).toBe(0);
    expect(progressPercent(5, -1)).toBe(0);
    expect(progressPercent(Number.NaN, 10)).toBe(0);
    expect(progressPercent(5, Number.POSITIVE_INFINITY)).toBe(0);
  });
});

describe('disclosure memory', () => {
  const store = () => {
    const data = new Map<string, string>();
    return {
      getItem: (k: string) => data.get(k) ?? null,
      setItem: (k: string, v: string) => void data.set(k, v),
    };
  };
  const blocked = {
    getItem: () => {
      throw new Error('SecurityError');
    },
    setItem: () => {
      throw new Error('SecurityError');
    },
  };

  it('remembers open and shut per id', () => {
    const s = store();
    expect(readDisclosureOpen('a', s)).toBe(false);
    writeDisclosureOpen('a', true, s);
    expect(readDisclosureOpen('a', s)).toBe(true);
    expect(readDisclosureOpen('b', s)).toBe(false);
    writeDisclosureOpen('a', false, s);
    expect(readDisclosureOpen('a', s)).toBe(false);
    expect(disclosureKey('a')).toBe('sn-disclosure:a');
  });

  it('works without storage at all', () => {
    expect(readDisclosureOpen('a', blocked)).toBe(false);
    expect(() => writeDisclosureOpen('a', true, blocked)).not.toThrow();
    // Node has no sessionStorage global; the default lookup must not throw either.
    expect(readDisclosureOpen('a')).toBe(false);
    expect(() => writeDisclosureOpen('a', true)).not.toThrow();
  });
});
