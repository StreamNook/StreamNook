// Runs every case in utils/accolades.cases.json through the wall and compares
// the whole output, item by item and key by key. The phones' Rust code and the
// streamnook.app member page run the same file, so a rule changed on one side
// only fails here, there or on the site.
import { describe, expect, it } from 'vitest';
import cases from '../../utils/accolades.cases.json';
import { buildAccoladeWall, localDay, type AccoladeWall, type WallInput } from './accoladeWall';

interface Case {
  name: string;
  own: boolean;
  input: Omit<WallInput, 'own'>;
  expect: AccoladeWall;
}

const file = cases as unknown as { v: number; cases: Case[] };

describe('accolade wall cases', () => {
  it('reads a plausible cases file', () => {
    expect(file.v).toBe(1);
    expect(file.cases.length).toBeGreaterThanOrEqual(16);
  });

  for (const c of file.cases) {
    it(c.name, () => {
      expect(buildAccoladeWall({ ...c.input, own: c.own })).toStrictEqual(c.expect);
    });
  }
});

describe('the wall inputs', () => {
  const quiet = {
    own: true,
    messages: 0,
    hours: 0,
    streams: 0,
    drops: 0,
    points: 0,
    cosmetics: 0,
    years: 0,
    member_number: 500,
    saved: [] as string[],
    hidden: [] as string[],
    today: '',
    created_at: null,
  };

  it('lights nothing from a day that is not a date', () => {
    for (const today of ['', 'today', '2026-13-01', '2026-11-5']) {
      const wall = buildAccoladeWall({ ...quiet, today });
      expect(wall.items.filter((i) => i.earned)).toEqual([]);
    }
  });

  it('writes a local date as year-month-day with two digits each', () => {
    expect(localDay(new Date(2026, 0, 5))).toBe('2026-01-05');
    expect(localDay(new Date(2026, 10, 25))).toBe('2026-11-25');
  });
});
