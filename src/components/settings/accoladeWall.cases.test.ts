// Runs every public case in utils/accolades.cases.json through the wall and
// compares the whole output, item by item and key by key. Rust and the
// streamnook.app member page run the same file, so a rule changed on one side
// only fails here, there or on the site. The own cases in the file are Rust's:
// your own wall is built there (own_accolades), not here.
import { describe, expect, it } from 'vitest';
import cases from '../../utils/accolades.cases.json';
import { buildAccoladeWall, type AccoladeWall, type WallInput } from './accoladeWall';

interface Case {
  name: string;
  own: boolean;
  // The file's inputs also carry `today` and `created_at`, which only an own wall reads.
  input: WallInput & { today?: string; created_at?: string | null };
  expect: AccoladeWall;
}

const file = cases as unknown as { v: number; cases: Case[] };
const publicCases = file.cases.filter((c) => !c.own);

describe('public accolade wall cases', () => {
  it('reads a plausible cases file', () => {
    expect(file.v).toBe(1);
    expect(file.cases.length).toBeGreaterThanOrEqual(16);
    expect(publicCases.length).toBeGreaterThanOrEqual(10);
  });

  it('leaves the own cases to Rust: they are in the file and never built here', () => {
    expect(file.cases.some((c) => c.own)).toBe(true);
    expect(publicCases.every((c) => !c.own)).toBe(true);
  });

  for (const c of publicCases) {
    it(c.name, () => {
      expect(buildAccoladeWall(c.input)).toStrictEqual(c.expect);
    });
  }
});

describe('the wall inputs', () => {
  const quiet: WallInput = {
    messages: 0,
    hours: 0,
    streams: 0,
    drops: 0,
    points: 0,
    cosmetics: 0,
    years: 0,
    member_number: 500,
    saved: [],
    hidden: [],
  };

  it('lights nothing from the clock: only the stored set lights a season or Cake Day', () => {
    expect(buildAccoladeWall(quiet).items.filter((i) => i.earned)).toEqual([]);
    const saved = buildAccoladeWall({ ...quiet, saved: ['halloween', 'cake_day'] });
    expect(saved.items.filter((i) => i.earned).map((i) => i.id)).toEqual(['halloween', 'cake_day']);
  });

  it('lists no secret that is not earned, and never prints its how-to', () => {
    const wall = buildAccoladeWall(quiet);
    expect(wall.items.some((i) => i.secret && !i.earned)).toBe(false);
    const held = buildAccoladeWall({ ...quiet, saved: ['insomniac'] });
    const item = held.items.find((i) => i.id === 'insomniac');
    expect(item?.secret).toBe(true);
    expect(item?.hint).toBe('A hidden accolade.');
  });
});
