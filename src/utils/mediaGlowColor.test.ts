import { describe, expect, it } from 'vitest';

import {
  ATTACK_MS,
  cssColour,
  follow,
  frameColours,
  lift,
  RELEASE_MS,
  SEGMENTS,
  VH,
  VW,
  type Rgb,
} from './mediaGlowColor';

/** A frame of solid colour, in the readback's exact shape. */
function solid(r: number, g: number, b: number): Uint8ClampedArray {
  const d = new Uint8ClampedArray(VW * VH * 4);
  for (let i = 0; i < d.length; i += 4) {
    d[i] = r;
    d[i + 1] = g;
    d[i + 2] = b;
    d[i + 3] = 255;
  }
  return d;
}

/** A frame painted by a function of pixel position, so a test can describe the
 *  picture rather than the buffer. */
function paint(f: (x: number, y: number) => [number, number, number]): Uint8ClampedArray {
  const d = new Uint8ClampedArray(VW * VH * 4);
  for (let y = 0; y < VH; y++) {
    for (let x = 0; x < VW; x++) {
      const [r, g, b] = f(x, y);
      const i = (y * VW + x) * 4;
      d[i] = r;
      d[i + 1] = g;
      d[i + 2] = b;
      d[i + 3] = 255;
    }
  }
  return d;
}

const lum = (c: Rgb) => 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
const sat = (c: Rgb) => {
  const max = Math.max(c.r, c.g, c.b);
  const min = Math.min(c.r, c.g, c.b);
  return max === 0 ? 0 : (max - min) / max;
};

describe('lift', () => {
  it('stays brighter for a brighter source, all the way up', () => {
    // The property the whole effect rests on, and the one a gain-plus-clamp
    // quietly breaks: above the clamp every scene emits the same light, so a
    // lit room and a snow field look identical and the light stops tracking
    // the picture. Checked across the full range, not at two points.
    let prev = -1;
    for (let v = 0; v <= 1.0001; v += 0.05) {
      const here = lum(lift(v, v, v));
      expect(here).toBeGreaterThan(prev);
      prev = here;
    }
  });

  it('gives a black frame a floor rather than switching the light off', () => {
    const l = lum(lift(0, 0, 0));
    expect(l).toBeGreaterThan(0.02);
    expect(l).toBeLessThan(0.12);
  });

  it('keeps a near-grey source near-grey instead of inventing a hue', () => {
    // Real thumbnails and real night scenes measure around s = 0.04. Forcing
    // those up to a usable saturation does not reveal a colour, it invents one
    // — and a different one every frame, which is visible as flicker.
    const c = lift(0.40, 0.41, 0.42);
    expect(sat(c)).toBeLessThan(0.15);
  });

  it('lifts a muddy colour into something that reads as light', () => {
    // An average of real pixels is always duller than the picture looks.
    const src = { r: 0.30, g: 0.45, b: 0.22 };
    const out = lift(src.r, src.g, src.b);
    expect(sat(out)).toBeGreaterThan(sat(src));
  });

  it('never leaves the 0..1 range it has to be rendered in', () => {
    for (const [r, g, b] of [
      [0, 0, 0],
      [1, 1, 1],
      [1, 0, 0],
      [0, 1, 0],
      [0, 0, 1],
      [0.02, 0.9, 0.5],
    ]) {
      const c = lift(r, g, b);
      for (const ch of [c.r, c.g, c.b]) {
        expect(ch).toBeGreaterThanOrEqual(0);
        expect(ch).toBeLessThanOrEqual(1);
      }
    }
  });
});

describe('frameColours', () => {
  it('reads the top and bottom of the picture independently', () => {
    // Red sky over blue ground. The bands have to come back as the sky and the
    // ground, not as two shades of the average.
    const d = paint((_x, y) => (y < VH / 2 ? [210, 40, 40] : [40, 60, 210]));
    const c = frameColours(d);
    expect(c.top[0].r).toBeGreaterThan(c.top[0].b);
    expect(c.bottom[0].b).toBeGreaterThan(c.bottom[0].r);
  });

  it('gives every segment its own column of the picture', () => {
    // A red left half and a blue right half. If a segment averaged the whole
    // width, all eight would come back the same and the light could not
    // correspond to what is above it — which is exactly why a one-colour edge
    // reads as a painted strip.
    const d = paint((x) => (x < VW / 2 ? [210, 40, 40] : [40, 60, 210]));
    const c = frameColours(d);
    expect(c.top).toHaveLength(SEGMENTS);
    expect(c.top[0].r).toBeGreaterThan(c.top[0].b);
    expect(c.top[SEGMENTS - 1].b).toBeGreaterThan(c.top[SEGMENTS - 1].r);
  });

  it('follows the scene down when the picture goes dark', () => {
    const brightLum = lum(frameColours(solid(200, 200, 200)).overall);
    const darkLum = lum(frameColours(solid(18, 18, 18)).overall);
    expect(darkLum).toBeLessThan(brightLum * 0.6);
  });

  it('ignores the middle of the frame, where the subject usually is', () => {
    // A vivid band across the centre must not reach either edge's reading.
    const plain = frameColours(solid(60, 60, 60));
    const withSubject = frameColours(
      paint((_x, y) => (y > VH / 3 && y < (VH * 2) / 3 ? [255, 0, 255] : [60, 60, 60])),
    );
    expect(withSubject.top[0]).toEqual(plain.top[0]);
    expect(withSubject.bottom[0]).toEqual(plain.bottom[0]);
  });
});

describe('follow', () => {
  const dark = { r: 0.1, g: 0.1, b: 0.1 };
  const bright = { r: 0.9, g: 0.9, b: 0.9 };

  it('rises faster than it falls', () => {
    // Asymmetry is most of what makes the light feel attached to the content:
    // a cut to a bright scene has to arrive with the cut, but one dark frame
    // must not blink the light off. A symmetric follower strobes on cuts.
    const up = follow(dark, bright, 60, ATTACK_MS, RELEASE_MS);
    const down = follow(bright, dark, 60, ATTACK_MS, RELEASE_MS);
    const rose = (up.r - dark.r) / (bright.r - dark.r);
    const fell = (bright.r - down.r) / (bright.r - dark.r);
    expect(rose).toBeGreaterThan(fell * 2);
  });

  it('moves the same distance per unit of time however the frames land', () => {
    // Expressed against elapsed time rather than per sample, so a stream that
    // drops to 15fps does not also get a slower light. Two 50ms steps have to
    // land where one 100ms step does.
    const once = follow(dark, bright, 100, ATTACK_MS, RELEASE_MS);
    const twice = follow(
      follow(dark, bright, 50, ATTACK_MS, RELEASE_MS),
      bright,
      50,
      ATTACK_MS,
      RELEASE_MS,
    );
    expect(twice.r).toBeCloseTo(once.r, 6);
  });

  it('approaches the target without overshooting it', () => {
    let c = dark;
    for (let i = 0; i < 200; i++) c = follow(c, bright, 60, ATTACK_MS, RELEASE_MS);
    expect(c.r).toBeGreaterThan(0.89);
    expect(c.r).toBeLessThanOrEqual(bright.r);
  });
});

describe('cssColour', () => {
  it('emits a colour the style system will accept', () => {
    expect(cssColour({ r: 0, g: 0.5, b: 1 })).toBe('rgb(0 128 255)');
  });

  it('clamps rather than emitting an out-of-range colour', () => {
    // Nothing should produce these, but a single bad frame must not write a
    // value that makes the whole gradient invalid and blanks the light.
    expect(cssColour({ r: -1, g: 2, b: 0.5 })).toBe('rgb(0 255 128)');
  });
});
