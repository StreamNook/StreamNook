// Numbers drawn in a slot that cannot grow (a square button, a round badge).
// Each formatter has a fixed most-characters, so the slot is sized for that
// once and the number never clips, whatever value arrives.

/** The most characters any formatter here returns. */
export const FIT_MAX_CHARS = 3;

/**
 * Seconds left, in at most three characters: "45", "120", "17m", "3h", "2d".
 * Rounds up, so a wait never reads shorter than it is. Nothing usable
 * (negative, NaN, Infinity) reads "0".
 */
export function fitSeconds(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return '0';
  const s = Math.ceil(seconds);
  if (s < 1000) return String(s);
  const minutes = Math.ceil(s / 60);
  if (minutes < 100) return `${minutes}m`;
  const hours = Math.ceil(s / 3600);
  if (hours < 100) return `${hours}h`;
  return `${Math.min(99, Math.ceil(s / 86400))}d`;
}

/**
 * A count in at most three characters: "7", "99", then "99+". `cap` lowers
 * the ceiling for a smaller slot ("9+" in a 16 px dot).
 */
export function fitCount(count: number, cap: 9 | 99 = 99): string {
  if (!Number.isFinite(count) || count <= 0) return '0';
  const n = Math.floor(count);
  return n > cap ? `${cap}+` : String(n);
}

/** The most characters `fitAmount` returns. */
export const FIT_AMOUNT_MAX_CHARS = 4;

/**
 * A size that keeps its scale, in at most four characters: "640", "2K",
 * "45K", "999K", "12M". Rounds down, so it never claims more than there is;
 * the exact number belongs in the tooltip.
 */
export function fitAmount(count: number): string {
  if (!Number.isFinite(count) || count <= 0) return '0';
  const n = Math.floor(count);
  if (n < 1000) return String(n);
  for (const [size, unit] of [[1e3, 'K'], [1e6, 'M'], [1e9, 'B']] as const) {
    const whole = Math.floor(n / size);
    if (whole < 1000) return `${whole}${unit}`;
  }
  return '999B';
}
