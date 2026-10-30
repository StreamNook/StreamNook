// Your songs' time groups and list layout. Pure (type imports only), so it is
// tested without a page. The page makes the bounds (local time is exact
// here), Rust counts every matched song into them (history.rs `page` answers
// `groups`), and this lays the list out: the Latest card, a header per group
// and the rows, every item's height known up front, so nothing above the
// viewport moves as pages land.

import type { SongHistoryChange } from '../../utils/songHistory';

/** Where one group starts: `bound` indexes the bounds sent (their count for
 *  "Older"), `start` is the first matched song in it. */
export interface SongGroup {
  bound: number;
  start: number;
}

/** Group bounds in ms, strictly newest first, and each group's name:
 *  `labels[i]` names the songs heard at or after `ms[i]` (and before
 *  `ms[i - 1]`); the last label, "Older", the songs before every bound. */
export interface Bounds {
  ms: number[];
  labels: string[];
}

/** Months named on their own before the list counts in years. */
const MONTHS_BACK = 24;
/** Years named before the rest are "Older". */
const YEARS_BACK = 10;

const MONTH_NAMES = [
  'January',
  'February',
  'March',
  'April',
  'May',
  'June',
  'July',
  'August',
  'September',
  'October',
  'November',
  'December',
];

/**
 * The bounds for `now`, in local time and calendar days (a day across a clock
 * change is still one day): today, yesterday, the last seven days, the rest of
 * this month, each earlier month for two years ("September", "September 2025"
 * outside this year), then each earlier year for ten. A bound that is not
 * below the one before it is dropped (early in a month, "Earlier this month"
 * has no days of its own).
 */
export function boundsFor(now: Date): Bounds {
  const y = now.getFullYear();
  const m = now.getMonth();
  const d = now.getDate();
  const ms: number[] = [];
  const labels: string[] = [];
  const add = (at: Date, label: string) => {
    const t = at.getTime();
    if (ms.length && t >= ms[ms.length - 1]) return;
    ms.push(t);
    labels.push(label);
  };
  add(new Date(y, m, d), 'Today');
  add(new Date(y, m, d - 1), 'Yesterday');
  add(new Date(y, m, d - 6), 'This week');
  add(new Date(y, m, 1), 'Earlier this month');
  for (let i = 1; i <= MONTHS_BACK; i++) {
    const at = new Date(y, m - i, 1);
    const name = MONTH_NAMES[at.getMonth()];
    add(at, at.getFullYear() === y ? name : `${name} ${at.getFullYear()}`);
  }
  for (let i = 0; i <= YEARS_BACK; i++) add(new Date(y - i, 0, 1), String(y - i));
  labels.push('Older');
  return { ms, labels };
}

/** Whether two sets of bounds are the same values (bounds keep their
 *  identity until their values change, so a list does not ask again). */
export function sameBounds(a: Bounds, b: Bounds): boolean {
  return (
    a.ms.length === b.ms.length &&
    a.ms.every((v, i) => v === b.ms[i]) &&
    a.labels.length === b.labels.length &&
    a.labels.every((v, i) => v === b.labels[i])
  );
}

/** Ms from `now` to the next local midnight, when the bounds move on. */
export function msToNextDay(now: Date): number {
  return new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1).getTime() - now.getTime();
}

// ---------------------------------------------------------------- layout

export const LATEST_HEIGHT = 112;
export const HEADER_HEIGHT = 34;
/** A row's height in each shell (SongsPanel's ROW). */
export const ROW_HEIGHT = { modal: 76, screen: 80 } as const;

export type ListItem = { kind: 'latest' } | { kind: 'header'; label: string; count: number } | { kind: 'row'; row: number };

export interface Layout {
  /** Items in the list. */
  count: number;
  item(i: number): ListItem;
  size(i: number): number;
  /** The item showing matched song `row` (row 0 is the Latest card when it shows). */
  itemOfRow(row: number): number;
  /** The list's whole height. */
  total(): number;
  /** Where item `i` starts, from the list's top. */
  offset(i: number): number;
  /** The item at `px` from the list's top. */
  itemAt(px: number): number;
}

/** A run of rows, under a header or not, and where it starts. */
interface Segment {
  label: string | null;
  rowStart: number;
  rowEnd: number;
  itemStart: number;
  offsetStart: number;
}

/** The last segment whose `key` is at or before `v` (segments are sorted). */
function segmentAt(segments: Segment[], v: number, key: (s: Segment) => number): number {
  let lo = 0;
  let hi = segments.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (key(segments[mid]) <= v) lo = mid;
    else hi = mid - 1;
  }
  return lo;
}

/**
 * The list's items: with `latest` (no search, at least one song) the Latest
 * card first, showing row 0; then per group its header and its rows. The group
 * holding row 0 starts at row 1 under the Latest card, and a group left with
 * no rows is dropped. Without groups (a search, or no bounds) the rows stand
 * alone.
 */
export function makeLayout(
  matched: number,
  groups: SongGroup[],
  labels: string[],
  latest: boolean,
  rowHeight: number,
): Layout {
  const rows = Math.max(0, matched);
  const showLatest = latest && rows > 0;
  const first = showLatest ? 1 : 0;
  const segments: Segment[] = [];
  let item = showLatest ? 1 : 0;
  let offset = showLatest ? LATEST_HEIGHT : 0;
  const push = (label: string | null, start: number, end: number) => {
    const rowStart = Math.max(first, start);
    const rowEnd = Math.min(rows, end);
    if (rowEnd <= rowStart) return;
    segments.push({ label, rowStart, rowEnd, itemStart: item, offsetStart: offset });
    const n = rowEnd - rowStart;
    item += (label === null ? 0 : 1) + n;
    offset += (label === null ? 0 : HEADER_HEIGHT) + n * rowHeight;
  };
  const sorted = [...groups].sort((a, b) => a.start - b.start);
  if (!sorted.length) push(null, first, rows);
  else {
    // Rows before the first group (none when Rust grouped every song) stand alone.
    push(null, first, sorted[0].start);
    sorted.forEach((g, i) => push(labels[g.bound] ?? '', g.start, i + 1 < sorted.length ? sorted[i + 1].start : rows));
  }
  const count = item;
  const totalHeight = offset;

  const inSegment = (i: number): { s: Segment; at: number } => {
    const s = segments[segmentAt(segments, i, (x) => x.itemStart)];
    return { s, at: i - s.itemStart };
  };
  const itemOf = (i: number): ListItem => {
    if (showLatest && i === 0) return { kind: 'latest' };
    if (!segments.length) return { kind: 'row', row: 0 };
    const { s, at } = inSegment(i);
    if (s.label !== null) {
      if (at === 0) return { kind: 'header', label: s.label, count: s.rowEnd - s.rowStart };
      return { kind: 'row', row: s.rowStart + at - 1 };
    }
    return { kind: 'row', row: s.rowStart + at };
  };
  return {
    count,
    item: itemOf,
    size(i) {
      const it = itemOf(i);
      return it.kind === 'latest' ? LATEST_HEIGHT : it.kind === 'header' ? HEADER_HEIGHT : rowHeight;
    },
    itemOfRow(row) {
      if (showLatest && row <= 0) return 0;
      if (!segments.length) return 0;
      const s = segments[segmentAt(segments, row, (x) => x.rowStart)];
      const r = Math.min(Math.max(row, s.rowStart), s.rowEnd - 1);
      return s.itemStart + (s.label === null ? 0 : 1) + (r - s.rowStart);
    },
    total: () => totalHeight,
    offset(i) {
      if (i <= 0 || !segments.length) return showLatest && i > 0 ? LATEST_HEIGHT : 0;
      if (i >= count) return totalHeight;
      const { s, at } = inSegment(i);
      if (s.label === null) return s.offsetStart + at * rowHeight;
      return at === 0 ? s.offsetStart : s.offsetStart + HEADER_HEIGHT + (at - 1) * rowHeight;
    },
    itemAt(px) {
      if (count === 0 || px <= 0) return 0;
      if (px >= totalHeight) return count - 1;
      if (showLatest && px < LATEST_HEIGHT) return 0;
      if (!segments.length) return 0;
      const s = segments[segmentAt(segments, px, (x) => x.offsetStart)];
      const into = px - s.offsetStart;
      if (s.label === null) return s.itemStart + Math.min(s.rowEnd - s.rowStart - 1, Math.floor(into / rowHeight));
      if (into < HEADER_HEIGHT) return s.itemStart;
      return s.itemStart + 1 + Math.min(s.rowEnd - s.rowStart - 1, Math.floor((into - HEADER_HEIGHT) / rowHeight));
    },
  };
}

/**
 * How far the list must scroll so matched song `row` stays where it was when
 * the layout changes around it with the same songs in the same order (the day
 * moved on and the headers above it changed).
 */
export function anchorDelta(before: Layout, after: Layout, row: number): number {
  return after.offset(after.itemOfRow(row)) - before.offset(before.itemOfRow(row));
}

// ---------------------------------------------------------------- held changes

/**
 * Changes held while the list is scrolled away from its top. A song filed to
 * the top (new, or heard again) would move every row on screen down, and its
 * old place is not known when its page is not loaded, so it waits: the rows on
 * screen stay, a quiet "New song" pill counts what waits, and the pill or a
 * scroll back to the top applies it all and goes to the top. Other changes
 * apply at once (colors, or rows moving up), unless something already waits:
 * asking for the rows again would bring the held songs in with them.
 */
export interface HeldChanges {
  /** Songs filed to the top while scrolled, oldest first. */
  filed: string[];
  /** Another change came while holding: applying asks for the rows again. */
  more: boolean;
}

export const NOTHING_HELD: HeldChanges = { filed: [], more: false };

/** What to do with the list after a change: the held state, and whether to
 *  ask for the rows on screen again now. */
export interface HeldStep {
  held: HeldChanges;
  apply: boolean;
}

/**
 * A change from Rust, with the list scrolled away from its top or not.
 * `toTop` says whether a `filed` change put its song at the top of the list
 * (filedToTop); a song changed in place (its links looked up again) applies
 * at once.
 */
export function holdChange(held: HeldChanges, change: SongHistoryChange, scrolled: boolean, toTop = true): HeldStep {
  if (change.kind === 'cleared') return { held: NOTHING_HELD, apply: true };
  const holding = held.filed.length > 0;
  if (!scrolled) return { held: NOTHING_HELD, apply: true };
  if (change.kind === 'filed') {
    if (!toTop && !holding) return { held, apply: true };
    if (!toTop) return { held: { ...held, more: true }, apply: false };
    return { held: { filed: [...held.filed.filter((k) => k !== change.key), change.key], more: held.more }, apply: false };
  }
  if (!holding) return { held, apply: true };
  if (change.kind === 'removed' && held.filed.includes(change.key)) {
    const filed = held.filed.filter((k) => k !== change.key);
    // The one waiting song is gone again: what is left only moves rows up.
    if (!filed.length) return { held: NOTHING_HELD, apply: true };
    return { held: { filed, more: true }, apply: false };
  }
  return { held: { ...held, more: true }, apply: false };
}

/** The pill pressed, or the list back at its top: apply what waits. */
export function releaseHeld(held: HeldChanges): HeldStep {
  return { held: NOTHING_HELD, apply: held.filed.length > 0 || held.more };
}

/** Songs the pill counts. */
export const heldCount = (held: HeldChanges): number => held.filed.length;

/** The list's first row, before and after a change, as far as it matters here. */
export interface TopRow {
  key: string;
  last_ms: number;
}

/** Whether filing `key` put it at the top: the first row is now that song and
 *  was another, or it is the same song heard again. */
export function filedToTop(key: string, before: TopRow | null, after: TopRow | null): boolean {
  if (!after || after.key !== key) return false;
  return !before || before.key !== key || before.last_ms !== after.last_ms;
}
