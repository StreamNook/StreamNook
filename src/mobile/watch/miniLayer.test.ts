import { describe, expect, it } from 'vitest';
import { miniLayerTransform } from './miniLayer';

/** Where a point of the layer lands on screen under a transform built by
 *  miniLayerTransform (origin top-left: scale first, then translate). */
function place(transform: string, x: number, y: number) {
  if (transform === 'none') return { x, y };
  const m = /translate\(([-\d.e]+)px, ([-\d.e]+)px\) scale\(([-\d.e]+)\)/.exec(transform);
  if (!m) throw new Error(`unexpected transform ${transform}`);
  const [tx, ty, k] = [Number(m[1]), Number(m[2]), Number(m[3])];
  return { x: tx + x * k, y: ty + y * k, k };
}

// A 360 px wide phone, the band 24 px down under the status bar, the mini box
// 200 px wide resting at (144, 575).
const SCALE = 200 / 360;
const BOX = { x: 144, y: 575 };
const TOP = 24;

describe('miniLayerTransform', () => {
  it('leaves the full player untouched', () => {
    expect(miniLayerTransform(0, SCALE, BOX.x, BOX.y, TOP)).toBe('none');
  });

  it('puts the band exactly on the mini box', () => {
    const at = place(miniLayerTransform(1, SCALE, BOX.x, BOX.y, TOP), 0, TOP);
    expect(at.x).toBeCloseTo(BOX.x, 6);
    expect(at.y).toBeCloseTo(BOX.y, 6);
    // 360 px of band at the box's scale is the box's 200 px.
    expect(360 * (at.k ?? 1)).toBeCloseTo(200, 6);
  });

  it('shrinks steadily on the way down', () => {
    let last = Infinity;
    for (let p = 0.1; p <= 1.0001; p += 0.1) {
      const { k } = place(miniLayerTransform(p, SCALE, BOX.x, BOX.y, TOP), 0, TOP);
      expect(k).toBeLessThan(last);
      last = k ?? 1;
    }
  });
});
