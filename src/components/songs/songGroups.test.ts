// The list's time groups are counted in local calendar days (a day across a
// clock change is still one day), laid out with every height known up front,
// and a song filed to the top while the list is scrolled waits for the "New
// song" pill instead of moving the rows on screen.
import { afterAll, describe, expect, it } from 'vitest';

// A zone with a clock change (the second Sunday of March, 2026-03-08).
const zone = process.env.TZ;
process.env.TZ = 'America/New_York';
afterAll(() => {
  process.env.TZ = zone;
});

import {
  anchorDelta,
  boundsFor,
  filedToTop,
  HEADER_HEIGHT,
  heldCount,
  holdChange,
  LATEST_HEIGHT,
  makeLayout,
  msToNextDay,
  NOTHING_HELD,
  releaseHeld,
  sameBounds,
  type Layout,
} from './songGroups';

const DAY = 86_400_000;
const at = (y: number, m: number, d: number) => new Date(y, m, d).getTime();

describe('time group bounds', () => {
  it('names today, yesterday, the week, the month, two years of months, then years', () => {
    const b = boundsFor(new Date(2026, 9, 29, 15, 30));
    expect(b.labels.length).toBe(b.ms.length + 1);
    expect(b.labels.slice(0, 6)).toEqual(['Today', 'Yesterday', 'This week', 'Earlier this month', 'September', 'August']);
    expect(b.ms.slice(0, 5)).toEqual([at(2026, 9, 29), at(2026, 9, 28), at(2026, 9, 23), at(2026, 9, 1), at(2026, 8, 1)]);
    expect(b.labels).toContain('January');
    expect(b.labels).toContain('December 2025');
    // 24 months back is October 2024; then the years before it.
    const last = b.labels.indexOf('October 2024');
    expect(b.labels.slice(last, last + 3)).toEqual(['October 2024', '2024', '2023']);
    expect(b.ms[last + 1]).toBe(at(2024, 0, 1));
    expect(b.labels.at(-2)).toBe('2016');
    expect(b.labels.at(-1)).toBe('Older');
    for (let i = 1; i < b.ms.length; i++) expect(b.ms[i]).toBeLessThan(b.ms[i - 1]);
    expect(b.ms.length).toBeLessThanOrEqual(64);
  });

  it('counts a week across a clock change in calendar days', () => {
    const b = boundsFor(new Date(2026, 2, 10, 9));
    expect(b.ms[2]).toBe(at(2026, 2, 4));
    // Six calendar days back holds one 23-hour day.
    expect(b.ms[0] - b.ms[2]).toBe(6 * DAY - 3_600_000);
  });

  it('drops the month when the week already reaches past its start', () => {
    const b = boundsFor(new Date(2026, 9, 3, 12));
    expect(b.labels.slice(0, 4)).toEqual(['Today', 'Yesterday', 'This week', 'September']);
    expect(b.ms[2]).toBe(at(2026, 8, 27));
  });

  it('names last year with its year in January', () => {
    const mid = boundsFor(new Date(2027, 0, 15, 12));
    expect(mid.labels.slice(0, 6)).toEqual(['Today', 'Yesterday', 'This week', 'Earlier this month', 'December 2026', 'November 2026']);
    const early = boundsFor(new Date(2027, 0, 3, 12));
    expect(early.labels.slice(0, 4)).toEqual(['Today', 'Yesterday', 'This week', 'December 2026']);
    expect(early.ms[2]).toBe(at(2026, 11, 28));
  });

  it('tells equal bounds apart from moved ones, and the time to midnight', () => {
    const now = new Date(2026, 9, 29, 23, 0);
    expect(sameBounds(boundsFor(now), boundsFor(new Date(2026, 9, 29, 1)))).toBe(true);
    expect(sameBounds(boundsFor(now), boundsFor(new Date(2026, 9, 30, 0, 0)))).toBe(false);
    expect(msToNextDay(now)).toBe(3_600_000);
    // The night the clocks go back is 25 hours long.
    expect(msToNextDay(new Date(2026, 10, 1, 0, 0))).toBe(25 * 3_600_000);
  });
});

const LABELS = ['Today', 'Yesterday', 'This week', 'Earlier this month', 'September', 'Older'];
const GROUPS = [
  { bound: 0, start: 0 },
  { bound: 1, start: 1 },
  { bound: 4, start: 3 },
];

function sizes(l: Layout): number[] {
  return Array.from({ length: l.count }, (_, i) => l.size(i));
}

describe('list layout', () => {
  it('puts the newest song in the Latest card and drops a group left empty', () => {
    const l = makeLayout(6, GROUPS, LABELS, true, 76);
    expect(Array.from({ length: l.count }, (_, i) => l.item(i))).toEqual([
      { kind: 'latest' },
      { kind: 'header', label: 'Yesterday', count: 2 },
      { kind: 'row', row: 1 },
      { kind: 'row', row: 2 },
      { kind: 'header', label: 'September', count: 3 },
      { kind: 'row', row: 3 },
      { kind: 'row', row: 4 },
      { kind: 'row', row: 5 },
    ]);
    expect(sizes(l)).toEqual([LATEST_HEIGHT, HEADER_HEIGHT, 76, 76, HEADER_HEIGHT, 76, 76, 76]);
    expect(l.total()).toBe(LATEST_HEIGHT + 2 * HEADER_HEIGHT + 5 * 76);
  });

  it('heads every group without the Latest card, and stands rows alone without groups', () => {
    const l = makeLayout(6, GROUPS, LABELS, false, 80);
    expect(l.count).toBe(9);
    expect(l.item(0)).toEqual({ kind: 'header', label: 'Today', count: 1 });
    expect(l.item(1)).toEqual({ kind: 'row', row: 0 });
    const plain = makeLayout(3, [], LABELS, false, 76);
    expect(Array.from({ length: plain.count }, (_, i) => plain.item(i))).toEqual([
      { kind: 'row', row: 0 },
      { kind: 'row', row: 1 },
      { kind: 'row', row: 2 },
    ]);
    expect(makeLayout(0, [], LABELS, true, 76).count).toBe(0);
    expect(makeLayout(1, [{ bound: 0, start: 0 }], LABELS, true, 76).count).toBe(1);
  });

  it('maps rows to items and offsets to items both ways', () => {
    for (const latest of [true, false]) {
      const l = makeLayout(6, GROUPS, LABELS, latest, 76);
      for (let row = 0; row < 6; row++) {
        const item = l.item(l.itemOfRow(row));
        expect(item).toEqual(latest && row === 0 ? { kind: 'latest' } : { kind: 'row', row });
      }
      let y = 0;
      for (let i = 0; i < l.count; i++) {
        expect(l.offset(i)).toBe(y);
        expect(l.itemAt(y)).toBe(i);
        expect(l.itemAt(y + l.size(i) - 1)).toBe(i);
        y += l.size(i);
      }
      expect(l.offset(l.count)).toBe(l.total());
    }
  });

  it('keeps a row in place when the day moves on and the headers above it change', () => {
    // Before midnight: three songs today, two yesterday. After it, all five
    // were yesterday: one header fewer above row 4.
    const before = makeLayout(5, [{ bound: 0, start: 0 }, { bound: 1, start: 3 }], LABELS, true, 76);
    const after = makeLayout(5, [{ bound: 1, start: 0 }], LABELS, true, 76);
    expect(anchorDelta(before, after, 4)).toBe(-HEADER_HEIGHT);
    expect(anchorDelta(before, after, 1)).toBe(0);
  });
});

describe('changes held while the list is scrolled', () => {
  const filed = (key: string) => ({ kind: 'filed' as const, key });

  it('holds a song filed to the top, counts it once, and applies it from the pill', () => {
    let step = holdChange(NOTHING_HELD, filed('a'), true);
    expect(step).toEqual({ held: { filed: ['a'], more: false }, apply: false });
    step = holdChange(step.held, filed('b'), true);
    step = holdChange(step.held, filed('a'), true);
    expect(step.held.filed).toEqual(['b', 'a']);
    expect(heldCount(step.held)).toBe(2);
    expect(releaseHeld(step.held)).toEqual({ held: NOTHING_HELD, apply: true });
    expect(releaseHeld(NOTHING_HELD).apply).toBe(false);
  });

  it('applies everything at once at the top of the list', () => {
    expect(holdChange(NOTHING_HELD, filed('a'), false)).toEqual({ held: NOTHING_HELD, apply: true });
    const held = holdChange(NOTHING_HELD, filed('a'), true).held;
    expect(holdChange(held, { kind: 'tinted', keys: ['x'] }, false)).toEqual({ held: NOTHING_HELD, apply: true });
  });

  it('applies colors, removals and in-place changes at once while nothing waits', () => {
    expect(holdChange(NOTHING_HELD, { kind: 'tinted', keys: ['x'] }, true).apply).toBe(true);
    expect(holdChange(NOTHING_HELD, { kind: 'removed', key: 'x' }, true).apply).toBe(true);
    expect(holdChange(NOTHING_HELD, filed('x'), true, false).apply).toBe(true);
    expect(holdChange(NOTHING_HELD, { kind: 'cleared' }, true)).toEqual({ held: NOTHING_HELD, apply: true });
  });

  it('holds other changes behind a waiting song, since asking again would bring it in', () => {
    const held = holdChange(NOTHING_HELD, filed('a'), true).held;
    expect(holdChange(held, { kind: 'tinted', keys: ['x'] }, true)).toEqual({ held: { filed: ['a'], more: true }, apply: false });
    expect(holdChange(held, { kind: 'removed', key: 'x' }, true).apply).toBe(false);
    expect(holdChange(held, filed('x'), true, false)).toEqual({ held: { filed: ['a'], more: true }, apply: false });
    expect(holdChange(held, { kind: 'cleared' }, true)).toEqual({ held: NOTHING_HELD, apply: true });
  });

  it('lets go of a waiting song removed again: only rows moving up are left', () => {
    const one = holdChange(NOTHING_HELD, filed('a'), true).held;
    expect(holdChange(one, { kind: 'removed', key: 'a' }, true)).toEqual({ held: NOTHING_HELD, apply: true });
    const two = holdChange(one, filed('b'), true).held;
    expect(holdChange(two, { kind: 'removed', key: 'a' }, true)).toEqual({ held: { filed: ['b'], more: true }, apply: false });
  });

  it('tells a song filed to the top from one changed in place', () => {
    const top = { key: 'old', last_ms: 10 };
    expect(filedToTop('new', top, { key: 'new', last_ms: 20 })).toBe(true);
    // Heard again while already first.
    expect(filedToTop('old', top, { key: 'old', last_ms: 30 })).toBe(true);
    // Its links looked up again: it stays where it was.
    expect(filedToTop('mid', top, top)).toBe(false);
    expect(filedToTop('old', top, top)).toBe(false);
    expect(filedToTop('a', null, { key: 'a', last_ms: 1 })).toBe(true);
    expect(filedToTop('a', null, null)).toBe(false);
  });
});
