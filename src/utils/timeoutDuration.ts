// A typed timeout length in seconds: plain seconds ("600") or Twitch's unit form ("30s", "10m",
// "1h30m", "2d", "1w"). Null when it is not a duration, so the caller can say so instead of
// guessing a length.
const UNIT_SECONDS: Record<string, number> = { s: 1, m: 60, h: 3600, d: 86400, w: 604800 };

export function parseTimeoutSeconds(raw: string): number | null {
  const text = raw.trim().toLowerCase();
  if (/^\d+$/.test(text)) {
    const n = Number(text);
    return n > 0 ? n : null;
  }
  if (!/^(\d+[smhdw])+$/.test(text)) return null;
  let total = 0;
  for (const [, num, unit] of text.matchAll(/(\d+)([smhdw])/g)) {
    total += Number(num) * UNIT_SECONDS[unit];
  }
  return total > 0 ? total : null;
}
