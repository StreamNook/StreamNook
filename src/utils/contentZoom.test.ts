import { describe, expect, it } from 'vitest';
import { contentBox, FULL, sameBox, zoomFor } from './contentZoom';

/** An RGBA frame, `fill(x, y)` giving each pixel's grey level. */
function frame(w: number, h: number, fill: (x: number, y: number) => number): Uint8ClampedArray {
  const d = new Uint8ClampedArray(w * h * 4);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const i = (y * w + x) * 4;
      const v = fill(x, y);
      d[i] = v;
      d[i + 1] = v;
      d[i + 2] = v;
      d[i + 3] = 255;
    }
  }
  return d;
}

describe('contentBox', () => {
  it('finds a co-host band across the middle of a 9:16 frame', () => {
    // Picture from row 17 to 46 of 64, the rest black: two feeds side by side.
    const d = frame(36, 64, (_, y) => (y >= 17 && y < 47 ? 140 : 8));
    const box = contentBox(d, 36, 64);
    expect(box.y0).toBeCloseTo(17 / 64, 3);
    expect(box.y1).toBeCloseTo(47 / 64, 3);
    expect(box.x0).toBe(0);
    expect(box.x1).toBe(1);
  });

  it('leaves a frame with no bars alone', () => {
    expect(contentBox(frame(36, 64, () => 120), 36, 64)).toEqual(FULL);
  });

  it('does not treat a dark ceiling as a letterbox', () => {
    // Dark only at the top: a real bar comes with its opposite.
    const d = frame(36, 64, (_, y) => (y < 12 ? 5 : 150));
    expect(contentBox(d, 36, 64)).toEqual(FULL);
  });

  it('does not zoom into a mostly black scene', () => {
    const d = frame(36, 64, (_, y) => (y >= 28 && y < 36 ? 200 : 6));
    expect(contentBox(d, 36, 64)).toEqual(FULL);
  });
});

describe('zoomFor', () => {
  it('fills a wide player with the middle band of a tall video', () => {
    // A 720x1280 stream in a 1690x1060 player: drawn 596x1060, band 0.27..0.73.
    const t = zoomFor({ x0: 0, y0: 0.27, x1: 1, y1: 0.73 }, 1690, 1060, 720, 1280);
    const s = Number(t.match(/scale\(([\d.]+)\)/)?.[1]);
    // Height-limited: 1060 / (0.46 * 1060) is about 2.17.
    expect(s).toBeGreaterThan(2.1);
    expect(s).toBeLessThan(2.2);
    // A centred band needs no translation.
    expect(t.startsWith('translate(0.0px, 0.0px)') || t.startsWith('translate(-0.0px, -0.0px)')).toBe(true);
  });

  it('is the identity for the full frame', () => {
    expect(zoomFor(FULL, 1690, 1060, 720, 1280)).toBe('');
  });

  it('never magnifies past its cap', () => {
    const t = zoomFor({ x0: 0.45, y0: 0.45, x1: 0.55, y1: 0.55 }, 1000, 1000, 1000, 1000);
    expect(t).toContain('scale(3.000)');
  });

  it('treats near-identical boxes as the same', () => {
    expect(sameBox({ x0: 0, y0: 0.27, x1: 1, y1: 0.73 }, { x0: 0, y0: 0.28, x1: 1, y1: 0.72 })).toBe(true);
    expect(sameBox({ x0: 0, y0: 0.27, x1: 1, y1: 0.73 }, FULL)).toBe(false);
  });
});
