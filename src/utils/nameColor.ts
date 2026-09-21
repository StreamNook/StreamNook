// Readable username colors.
//
// Twitch lets a chatter pick any color, and on a dark chat a navy or maroon
// name is a smudge. `hsl_loop` walks the color's lightness toward the readable
// side of a YIQ brightness check, a tenth of the remaining distance per step,
// until it passes (or twenty steps, whichever first). Hue and saturation are
// untouched, so the name still reads as "that person's color", just visible.
// On a light theme the same walk runs the other way.
//
// Pure, cached and cheap: chat calls this once per rendered name.

export type NameColorAdjustment = 'off' | 'hsl_loop';

const MAX_STEPS = 20;
const STEP = 0.1;
/** YIQ brightness at or above this reads as light. */
const LIGHT_THRESHOLD = 128;
const CACHE_LIMIT = 2000;
const cache = new Map<string, string>();

function parseHex(color: string): [number, number, number] | null {
  const m = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(color.trim());
  if (!m) return null;
  let h = m[1];
  if (h.length === 3) h = h[0] + h[0] + h[1] + h[1] + h[2] + h[2];
  return [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16)];
}

function isLight([r, g, b]: [number, number, number]): boolean {
  return (r * 299 + g * 587 + b * 114) / 1000 >= LIGHT_THRESHOLD;
}

function rgbToHsl([r, g, b]: [number, number, number]): [number, number, number] {
  const rn = r / 255;
  const gn = g / 255;
  const bn = b / 255;
  const max = Math.max(rn, gn, bn);
  const min = Math.min(rn, gn, bn);
  const l = (max + min) / 2;
  if (max === min) return [0, 0, l];
  const d = max - min;
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  let h: number;
  if (max === rn) h = (gn - bn) / d + (gn < bn ? 6 : 0);
  else if (max === gn) h = (bn - rn) / d + 2;
  else h = (rn - gn) / d + 4;
  return [h / 6, s, l];
}

function hueToRgb(p: number, q: number, t: number): number {
  let tt = t;
  if (tt < 0) tt += 1;
  if (tt > 1) tt -= 1;
  if (tt < 1 / 6) return p + (q - p) * 6 * tt;
  if (tt < 1 / 2) return q;
  if (tt < 2 / 3) return p + (q - p) * (2 / 3 - tt) * 6;
  return p;
}

function hslToRgb([h, s, l]: [number, number, number]): [number, number, number] {
  if (s === 0) {
    const v = Math.round(l * 255);
    return [v, v, v];
  }
  const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
  const p = 2 * l - q;
  return [
    Math.round(hueToRgb(p, q, h + 1 / 3) * 255),
    Math.round(hueToRgb(p, q, h) * 255),
    Math.round(hueToRgb(p, q, h - 1 / 3) * 255),
  ];
}

const toHex = (rgb: [number, number, number]): string =>
  '#' + rgb.map((v) => Math.max(0, Math.min(255, v)).toString(16).padStart(2, '0')).join('');

/**
 * The color a name should render in.
 *
 * `darkTheme`: whether the chat background is dark (names need to be light).
 * Anything that is not a hex color (a paint, a CSS variable, an empty string)
 * comes back untouched.
 */
export function adjustNameColor(
  color: string,
  mode: NameColorAdjustment,
  darkTheme: boolean,
): string {
  if (mode === 'off' || !color) return color;
  const key = `${darkTheme ? 'd' : 'l'}:${color}`;
  const hit = cache.get(key);
  if (hit !== undefined) return hit;
  const rgb = parseHex(color);
  if (!rgb) return color;
  let hsl = rgbToHsl(rgb);
  let out = rgb;
  for (let i = 0; i < MAX_STEPS && isLight(out) !== darkTheme; i++) {
    // Toward white by a tenth of what is left, or toward black by a tenth.
    const l = darkTheme ? 1 - (1 - STEP) * (1 - hsl[2]) : (1 - STEP) * hsl[2];
    hsl = [hsl[0], hsl[1], l];
    out = hslToRgb(hsl);
  }
  const result = toHex(out);
  if (cache.size >= CACHE_LIMIT) {
    const first = cache.keys().next().value;
    if (first !== undefined) cache.delete(first);
  }
  cache.set(key, result);
  return result;
}

/** Test seam: the brightness verdict the loop steers by. */
export const nameColorReadsLight = (color: string): boolean | null => {
  const rgb = parseHex(color);
  return rgb ? isLight(rgb) : null;
};
