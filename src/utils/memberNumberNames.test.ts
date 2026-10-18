import { describe, expect, it } from 'vitest';
import { memberNumberName } from './memberNumberNames';

/** Every number up to `max` with its name, skipping the ones without. */
const named = (max: number): Array<[number, string]> => {
  const out: Array<[number, string]> = [];
  for (let n = 1; n <= max; n++) {
    const name = memberNumberName(n);
    if (name !== null) out.push([n, name]);
  }
  return out;
};

describe('memberNumberName', () => {
  it('keeps every number that earned the accolade before', () => {
    for (const n of [69, 420, 666, 777, 1337]) expect(memberNumberName(n)).not.toBeNull();
  });

  it('gives each meme number its own words', () => {
    expect(memberNumberName(67)).toBe('Six Seven');
    expect(memberNumberName(69)).toBe('Nice');
    expect(memberNumberName(420)).toBe('Blaze It');
    expect(memberNumberName(666)).toBe('The Beast');
    expect(memberNumberName(777)).toBe('Jackpot');
    expect(memberNumberName(1337)).toBe('Leet');
  });

  it('reads "Nice" for 69 and nothing else', () => {
    expect(named(20000).filter(([, name]) => name === 'Nice')).toEqual([[69, 'Nice']]);
  });

  it('calls every other number ending in 67 close enough', () => {
    for (const n of [167, 267, 667, 1067, 9967]) expect(memberNumberName(n)).toBe('Six Seven-ish');
  });

  it('never repeats a name, apart from the near misses', () => {
    const names = named(20000)
      .map(([, name]) => name)
      .filter((name) => name !== 'Six Seven-ish');
    expect(new Set(names).size).toBe(names.length);
  });

  it('leaves ordinary numbers and missing numbers unnamed', () => {
    for (const n of [null, 0, -67, 1, 66, 68, 168, 670, 676, 1.5]) expect(memberNumberName(n)).toBeNull();
  });
});
