/**
 * Piecewise "logarithmic" scale for the broadcast timeline.
 *
 * The bar is split into equal-width sections. The rightmost section (the
 * stretch just before live) is drawn at 1x, and each section to its left
 * compresses time by another factor of two, up to 16x. So on a 4 h
 * broadcast the last ~8 minutes get a fifth of the bar (fine control near
 * live), while the first two hours share the leftmost fifth. The whole
 * broadcast always fits: the base unit is `total / (1 + 2 + 4 + ... )`.
 *
 * Both directions are exact inverses, so the playhead, the hover tooltip
 * and a drop all agree.
 */

/** Compression factor of each section, rightmost first. */
export const SECTION_FACTORS: readonly number[] = [1, 2, 4, 8, 16];

export interface BroadcastScale {
  /** Bar fraction (0 = broadcast start, 1 = live) for a broadcast time. */
  toFrac: (secs: number) => number;
  /** Broadcast time for a bar fraction. */
  toSecs: (frac: number) => number;
  /** Section boundaries as bar fractions with the seconds-behind-live each
   *  boundary represents, left to right (excludes the two ends). */
  ticks: Array<{ frac: number; behindSecs: number }>;
  /** The sections themselves, left to right, with their compression factor. */
  bands: Array<{ fromFrac: number; toFrac: number; factor: number }>;
}

export function broadcastScale(totalSecs: number): BroadcastScale {
  const total = Number.isFinite(totalSecs) && totalSecs > 0 ? totalSecs : 1;
  const n = SECTION_FACTORS.length;
  const sumFactors = SECTION_FACTORS.reduce((a, b) => a + b, 0);
  const base = total / sumFactors;
  const sectionWidth = 1 / n;
  // Section i (0 = rightmost) covers behind-live seconds
  // [start_i, start_i + base * factor_i], where start_i is the sum of the
  // spans of the sections to its right.
  const spans = SECTION_FACTORS.map((f) => base * f);
  const starts: number[] = [];
  let acc = 0;
  for (const s of spans) {
    starts.push(acc);
    acc += s;
  }

  const toFrac = (secs: number): number => {
    const behind = Math.min(total, Math.max(0, total - secs));
    for (let i = 0; i < n; i++) {
      if (behind <= starts[i] + spans[i] || i === n - 1) {
        const within = Math.min(1, Math.max(0, (behind - starts[i]) / spans[i]));
        // Right edge of section i sits at 1 - i * sectionWidth. Clamp so the
        // far end lands on an exact 0 instead of float dust.
        return Math.min(1, Math.max(0, 1 - i * sectionWidth - within * sectionWidth));
      }
    }
    return 0;
  };

  const toSecs = (frac: number): number => {
    const f = Math.min(1, Math.max(0, frac));
    const fromRight = 1 - f;
    let i = Math.min(n - 1, Math.floor(fromRight / sectionWidth));
    if (fromRight >= 1) i = n - 1;
    const within = Math.min(1, Math.max(0, (fromRight - i * sectionWidth) / sectionWidth));
    const behind = starts[i] + within * spans[i];
    const secs = total - behind;
    // The far left is exactly the broadcast start, never float dust above it.
    return secs < 1e-6 ? 0 : Math.min(total, secs);
  };

  const ticks = starts
    .slice(1)
    .map((behindSecs, idx) => ({ frac: 1 - (idx + 1) * sectionWidth, behindSecs }))
    .reverse();

  const bands = SECTION_FACTORS.map((factor, i) => ({
    fromFrac: 1 - (i + 1) * sectionWidth,
    toFrac: 1 - i * sectionWidth,
    factor,
  })).reverse();

  return { toFrac, toSecs, ticks, bands };
}

/** "8m" / "1h 02m" style label for a section boundary. */
export function formatBehindShort(behindSecs: number): string {
  const s = Math.max(0, Math.round(behindSecs));
  const h = Math.floor(s / 3600);
  const m = Math.round((s % 3600) / 60);
  if (h > 0) return m > 0 ? `${h}h ${m.toString().padStart(2, '0')}m` : `${h}h`;
  if (m > 0) return `${m}m`;
  return `${s}s`;
}
