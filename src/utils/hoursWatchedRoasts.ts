// Hours-watched "roast generator".
//
// A pool of one-liners that translate a member's lifetime watch hours into
// something absurd (hoursRoasts.json, read by the phones' Rust code too).
// Lines come in three tones and the picker weights them 60% roast, 30%
// neutral, 10% cute, never repeating the line shown last.
//
// Two kinds of line:
//   - templated: references exactly one derived unit token ({days}, {weeks},
//     {movies}, {seasons}, {flights}). The line is only eligible when that unit
//     rounds to 2 or more, so "0 full days" and "1 full days" are never printed.
//   - flavor: no token, always eligible.
//
// Under `low_hours_max` hours the unit math reads silly, so a separate
// low-hours set is used instead.
//
// The picker is pure: it takes its random source and the line shown last as
// arguments. It draws once for the low-hours set, and twice otherwise (the tone,
// then the line). Comparisons are brand-free and timeless on purpose (no show or
// movie names).
import data from './hoursRoasts.json';

export type RoastTone = 'roast' | 'neutral' | 'cute';

type Unit = 'days' | 'weeks' | 'movies' | 'seasons' | 'flights';

interface RoastLine {
  tone: RoastTone;
  text: string;
  // The single unit token this line interpolates. Omitted for flavor lines.
  requires?: Unit;
}

export interface PickedRoast {
  text: string;
  tone: RoastTone;
}

// Derived-unit divisors (hours / divisor, rounded):
//   days  = a full 24-hour day
//   weeks = a full-time 40-hour work week
//   movies = a ~2-hour feature film
//   seasons = a ~9-hour season of television (~22 episodes)
//   flights = a ~5-hour cross-country flight
const DIVISORS = data.divisors as Record<Unit, number>;
const POOL = data.pool as RoastLine[];
const LOW_HOURS: string[] = data.low_hours;
const LOW_HOURS_MAX: number = data.low_hours_max;

// A templated line needs at least this many of its unit, so a plural never reads "1 full days".
const MIN_UNIT = 2;

const grouped = new Intl.NumberFormat('en-US');

const computeUnits = (hours: number): Record<Unit, number> => ({
  days: Math.round(hours / DIVISORS.days),
  weeks: Math.round(hours / DIVISORS.weeks),
  movies: Math.round(hours / DIVISORS.movies),
  seasons: Math.round(hours / DIVISORS.seasons),
  flights: Math.round(hours / DIVISORS.flights),
});

const interpolate = (line: RoastLine, units: Record<Unit, number>): string => {
  if (!line.requires) return line.text;
  return line.text.replace(`{${line.requires}}`, grouped.format(units[line.requires] ?? 0));
};

/** The entry a random draw in [0, 1) lands on. */
const at = <T,>(list: T[], r: number): T => list[Math.min(list.length - 1, Math.floor(r * list.length))];

/**
 * Pick a roast line for the given lifetime watch hours. Weights tone
 * 60/30/10 (roast/neutral/cute), only offers templated lines whose unit
 * rounds to 2 or more, and avoids repeating `last`, the line returned before.
 */
export const pickHoursRoast = (hours: number, rand: () => number = Math.random, last: string | null = null): PickedRoast => {
  if (!hours || hours < LOW_HOURS_MAX) {
    const fresh = LOW_HOURS.filter((t) => t !== last);
    return { text: at(fresh.length ? fresh : LOW_HOURS, rand()), tone: 'roast' };
  }

  const units = computeUnits(hours);
  const eligible = POOL.filter((l) => !l.requires || (units[l.requires] ?? 0) >= MIN_UNIT);

  const r = rand();
  const tone: RoastTone = r < 0.6 ? 'roast' : r < 0.9 ? 'neutral' : 'cute';

  // Prefer the weighted tone, but never get stuck: fall back to any eligible
  // line, then ignore the no-repeat guard only as a last resort.
  let candidates = eligible.filter((l) => l.tone === tone && interpolate(l, units) !== last);
  if (!candidates.length) candidates = eligible.filter((l) => interpolate(l, units) !== last);
  if (!candidates.length) candidates = eligible;

  const line = at(candidates, rand());
  return { text: interpolate(line, units), tone: line.tone };
};
