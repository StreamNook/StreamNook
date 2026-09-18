/**
 * The colour arithmetic behind the immersive light. A frame of pixels goes in,
 * the colours for each edge come out; nothing here knows what a video element
 * is, which is why it lives apart from the hook that drives it.
 *
 * This answers a different question from `services::media_glow` in Rust, and
 * the two must not be collapsed into one. Rust answers "what colour IS this
 * stream" for a card: one representative colour, chosen as the modal bucket and
 * pinned into a legible lightness band so every card reads evenly. This answers
 * "what is on screen right now": the mean, tracking the scene's own brightness,
 * recomputed many times a second. A modal colour jumps between buckets frame to
 * frame, which on a card is invisible and on a light is flicker.
 */

/** Segments per edge. Eight is where the light stops reading as a few coloured
 *  blocks and starts reading as a strip: at four, anything moving across the
 *  frame drags a quarter of the band with it. Sixteen doubles the arithmetic to
 *  sit under a blur that smooths most of the difference away again. */
export const SEGMENTS = 8;

/** Readback size for video. Eight segments need enough columns to be distinct;
 *  this is four pixels per segment and six rows per edge strip. */
export const VW = 32;
export const VH = 18;

/** How fast the light follows the picture, as time constants in ms.
 *
 *  Asymmetric, and this is most of what makes it feel attached to the content:
 *  a cut to a bright scene has to arrive WITH the cut, but one dark frame in a
 *  bright scene must not blink the light off. A light that falls as fast as it
 *  rises strobes on anything with cuts in it. */
export const ATTACK_MS = 70;
export const RELEASE_MS = 240;

/** Motion-sensitive users keep the colour and lose the chase: it drifts toward
 *  the scene instead of following it. */
export const CALM_ATTACK_MS = 900;
export const CALM_RELEASE_MS = 1400;

/** Lift applied after averaging. Both curves are soft knees rather than a gain
 *  and a clamp, and that distinction is the whole reason this looks like light.
 *
 *  An average of real pixels always looks duller than the picture does, because
 *  averaging pulls toward grey — every ambilight lifts saturation for the same
 *  reason. `1 - (1 - s) ** SAT_POW` lifts the muddy middle hard while leaving a
 *  near-grey frame near-grey, so nothing invents a colour out of sensor noise.
 *
 *  Lightness matters even more. A gain with a ceiling looks fine until you
 *  notice every scene above mid-grey clamps to the same value, so a lit room
 *  and a snow field emit identical light and the whole thing stops tracking the
 *  picture — which is exactly what the still-image picker does (it pins every
 *  colour into 0.45-0.65) and exactly why that picker is wrong for this job.
 *  `FLOOR + range * (1 - e^-K*l)` is monotonic everywhere: brighter scene,
 *  brighter light, always. The floor keeps a night scene a faint presence
 *  rather than off; the curve flattens near the top instead of hitting a wall. */
const SAT_POW = 1.7;
const LUM_K = 3.0;
const LUM_FLOOR = 0.05;
const LUM_CEIL = 0.8;

export interface Rgb {
  r: number;
  g: number;
  b: number;
}

/** Perceived brightness, only ever used to compare two colours. */
export function lum(c: Rgb): number {
  return 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
}

export function hueChannel(p: number, q: number, t: number): number {
  let x = t;
  if (x < 0) x += 1;
  if (x > 1) x -= 1;
  if (x < 1 / 6) return p + (q - p) * 6 * x;
  if (x < 1 / 2) return q;
  if (x < 2 / 3) return p + (q - p) * (2 / 3 - x) * 6;
  return p;
}

/** Take a mean colour and make it read as light: lift the saturation the
 *  averaging cost it, and keep its own brightness rather than a fixed one. */
export function lift(r: number, g: number, b: number): Rgb {
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  const d = max - min;
  let s = 0;
  let h = 0;
  if (d > 0) {
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
    if (max === r) h = ((g - b) / d) % 6;
    else if (max === g) h = (b - r) / d + 2;
    else h = (r - g) / d + 4;
    h /= 6;
    if (h < 0) h += 1;
  }
  const s2 = 1 - Math.pow(1 - s, SAT_POW);
  const l2 = LUM_FLOOR + (LUM_CEIL - LUM_FLOOR) * (1 - Math.exp(-LUM_K * l));
  if (s2 === 0) return { r: l2, g: l2, b: l2 };
  const q = l2 < 0.5 ? l2 * (1 + s2) : l2 + s2 - l2 * s2;
  const p = 2 * l2 - q;
  return { r: hueChannel(p, q, h + 1 / 3), g: hueChannel(p, q, h), b: hueChannel(p, q, h - 1 / 3) };
}

/** The mean colour of one rectangle of the readback, lifted.
 *
 *  A plain mean, NOT the modal colour the still picker uses. Two different
 *  questions: a card asks what colour a stream is, where one representative
 *  colour beats an average that greys out; a strip asks what is on screen now,
 *  where the average is the honest answer and tracking the scene's brightness
 *  is most of the effect. A modal colour also jumps between buckets frame to
 *  frame, which on a strip is visible as flicker rather than as detail. */
export function cellColour(d: Uint8ClampedArray, x0: number, x1: number, y0: number, y1: number): Rgb {
  let r = 0;
  let g = 0;
  let b = 0;
  let n = 0;
  for (let y = y0; y < y1; y++) {
    let i = (y * VW + x0) * 4;
    for (let x = x0; x < x1; x++, i += 4) {
      r += d[i];
      g += d[i + 1];
      b += d[i + 2];
      n++;
    }
  }
  if (n === 0) return { r: 0, g: 0, b: 0 };
  return lift(r / n / 255, g / n / 255, b / n / 255);
}

/** One exponential step from `from` toward `to`.
 *
 *  Expressed against elapsed time rather than per sample, so the feel does not
 *  change when the frame rate does — a stream that drops to 15fps must not also
 *  get a slower light. */
export function follow(from: Rgb, to: Rgb, dt: number, attack: number, rise: number): Rgb {
  const a = 1 - Math.exp(-dt / (lum(to) >= lum(from) ? attack : rise));
  return {
    r: from.r + (to.r - from.r) * a,
    g: from.g + (to.g - from.g) * a,
    b: from.b + (to.b - from.b) * a,
  };
}

export function cssColour(c: Rgb): string {
  const v = (x: number) => Math.round(Math.min(1, Math.max(0, x)) * 255);
  return `rgb(${v(c.r)} ${v(c.g)} ${v(c.b)})`;
}

/** Everything one frame yields. Split from the readback so the whole
 *  per-frame extraction is one pure function over a pixel buffer: that is the
 *  part worth testing, and it needs neither a DOM nor a decoder to run.
 *
 *  Thirds rather than halves vertically: the middle of a frame is usually the
 *  subject and the least like either edge, so excluding it keeps the top and
 *  bottom readings distinct instead of both drifting toward the centre. */
export interface FrameColours {
  overall: Rgb;
  top: Rgb[];
  bottom: Rgb[];
}

export function frameColours(d: Uint8ClampedArray): FrameColours {
  const band = Math.max(1, Math.round(VH / 3));
  const out: FrameColours = { overall: cellColour(d, 0, VW, 0, VH), top: [], bottom: [] };
  for (let i = 0; i < SEGMENTS; i++) {
    const x0 = Math.floor((VW * i) / SEGMENTS);
    const x1 = Math.floor((VW * (i + 1)) / SEGMENTS);
    out.top.push(cellColour(d, x0, x1, 0, band));
    out.bottom.push(cellColour(d, x0, x1, VH - band, VH));
  }
  return out;
}
