// Runs hoursRoasts.cases.json (the phones' Rust code runs the same file) and
// checks the pool itself: a templated line is only offered once its unit
// reaches 1, and the line shown last is never offered again.
import { describe, expect, it } from 'vitest';
import cases from './hoursRoasts.cases.json';
import data from './hoursRoasts.json';
import { pickHoursRoast } from './hoursWatchedRoasts';

interface RoastCase {
  name: string;
  hours: number;
  rand: number[];
  last: string | null;
  expect: { text: string; tone: string };
}
const file = cases as unknown as { v: number; cases: RoastCase[] };

/** A random source that hands out the listed numbers in order and counts them. */
const sequence = (values: number[]) => {
  const queue = [...values];
  return { rand: () => (queue.length ? (queue.shift() as number) : Number.NaN), left: () => queue.length };
};

describe('hours roast cases', () => {
  for (const c of file.cases) {
    it(c.name, () => {
      const src = sequence(c.rand);
      expect(pickHoursRoast(c.hours, src.rand, c.last)).toStrictEqual(c.expect);
      // One draw for the low-hours set, two (the tone, then the line) otherwise.
      expect(src.left(), 'draws left over').toBe(0);
    });
  }

  it('covers the tone edges, the low-hours set and a line avoided', () => {
    const draws = file.cases.flatMap((c) => c.rand);
    for (const edge of [0.6, 0.9, 0.999]) expect(draws).toContain(edge);
    expect(file.cases.some((c) => c.hours === 1.6)).toBe(true);
    expect(file.cases.some((c) => c.last !== null)).toBe(true);
  });
});

describe('the roast pool', () => {
  const units = Object.keys(data.divisors);

  it('gives every templated line its one token and every other line none', () => {
    for (const line of data.pool) {
      const tokens = line.text.match(/\{[a-z]+\}/g) ?? [];
      if ('requires' in line) expect(tokens).toEqual([`{${line.requires}}`]);
      else expect(tokens).toEqual([]);
    }
  });

  it('knows every unit it asks for, and every tone', () => {
    for (const line of data.pool) {
      if ('requires' in line) expect(units).toContain(line.requires);
      expect(['roast', 'neutral', 'cute']).toContain(line.tone);
    }
  });

  it('never repeats a line', () => {
    const texts = [...data.pool.map((l) => l.text), ...data.low_hours];
    expect(new Set(texts).size).toBe(texts.length);
  });
});

describe('pickHoursRoast', () => {
  /** Every line the picker could return for these hours, rendered. */
  const reachable = (hours: number): Set<string> => {
    const out = new Set<string>();
    for (const r1 of [0.1, 0.7, 0.95]) {
      for (let k = 0; k < 100; k++) {
        const draws = [r1, k / 100];
        out.add(pickHoursRoast(hours, () => draws.shift() as number).text);
      }
    }
    return out;
  };

  it('offers a line that needs a unit only once the unit rounds to 2', () => {
    for (const hours of [2, 3, 11.9, 12, 20, 24, 35, 36, 47, 59, 100, 1000, 50000]) {
      const unit = Object.fromEntries(Object.entries(data.divisors).map(([k, d]) => [k, Math.round(hours / d)]));
      const allowed = new Set(
        data.pool
          .filter((l) => !('requires' in l) || unit[l.requires as string] >= 2)
          .map((l) => ('requires' in l ? l.text.replace(`{${l.requires}}`, unit[l.requires as string].toLocaleString('en-US')) : l.text)),
      );
      for (const text of reachable(hours)) expect(allowed.has(text), `${hours}h: ${text}`).toBe(true);
    }
  });

  it('never returns the line shown last while another is eligible', () => {
    for (const hours of [0, 1.5, 2, 30, 500]) {
      for (const text of reachable(hours)) {
        for (const r1 of [0.1, 0.7, 0.95]) {
          for (let k = 0; k < 100; k += 7) {
            const draws = [r1, k / 100];
            expect(pickHoursRoast(hours, () => draws.shift() as number, text).text).not.toBe(text);
          }
        }
      }
    }
  });

  it('draws from Math.random when no source is given', () => {
    expect(pickHoursRoast(500).text.length).toBeGreaterThan(0);
    expect(pickHoursRoast(0, undefined, null).tone).toBe('roast');
  });
});
