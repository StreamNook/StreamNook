import { describe, it, expect } from 'vitest';
import { pickStoryboardVariant, storyboardToPlyrThumbnails } from './storyboard';
import type { VodStoryboard, VodStoryboardVariant } from '../types';

/** The manifest Twitch serves for xqc VOD 2871438321 (43,691 s), probed
 *  2026-09-21: 200 frames at 219 s in two sheet sizes. */
const BASE = 'https://d1m7jfoe9zdc1j.cloudfront.net/4d854b049e2c157a3aba_xqc_321317436761_1789151438/storyboards/';
const LOW: VodStoryboardVariant = {
  quality: 'low',
  width: 160,
  height: 90,
  rows: 40,
  cols: 5,
  count: 200,
  interval_secs: 219,
  images: ['2871438321-low-0.jpg'],
};
const HIGH: VodStoryboardVariant = {
  quality: 'high',
  width: 220,
  height: 124,
  rows: 10,
  cols: 5,
  count: 200,
  interval_secs: 219,
  images: ['2871438321-high-0.jpg', '2871438321-high-1.jpg', '2871438321-high-2.jpg', '2871438321-high-3.jpg'],
};
const XQC: VodStoryboard = { base_url: BASE, variants: [LOW, HIGH] };

describe('pickStoryboardVariant', () => {
  it('takes the largest frame', () => {
    expect(pickStoryboardVariant(XQC)).toBe(HIGH);
  });

  it('falls back when the largest variant is unusable', () => {
    const sb: VodStoryboard = { base_url: BASE, variants: [LOW, { ...HIGH, count: 0 }] };
    expect(pickStoryboardVariant(sb)).toBe(LOW);
  });

  it('returns null with nothing usable', () => {
    expect(pickStoryboardVariant({ base_url: BASE, variants: [] })).toBeNull();
    expect(pickStoryboardVariant({ base_url: BASE, variants: [{ ...LOW, images: [] }] })).toBeNull();
  });
});

describe('storyboardToPlyrThumbnails', () => {
  it('lays every frame out on its sheet in Plyr sprite terms', () => {
    const sets = storyboardToPlyrThumbnails(HIGH, BASE, 43691);
    expect(sets).toHaveLength(1);
    const [set] = sets;
    expect(set.frames).toHaveLength(200);
    expect(set.height).toBe(124);
    expect(set.urlPrefix).toBe(BASE);
    // Frame 7: sheet 0, second row, third column.
    expect(set.frames[7]).toEqual({
      startTime: 7 * 219,
      endTime: 8 * 219,
      text: '2871438321-high-0.jpg',
      x: 440,
      y: 124,
      w: 220,
      h: 124,
    });
    // Frame 57: 50 per sheet, so sheet 1, index 7 again.
    expect(set.frames[57].text).toBe('2871438321-high-1.jpg');
    expect(set.frames[57].x).toBe(440);
    expect(set.frames[57].y).toBe(124);
    expect(set.frames[57].startTime).toBe(12483);
  });

  it('stretches the last frame to the duration so the tail has a picture', () => {
    const [set] = storyboardToPlyrThumbnails(HIGH, BASE, 43691);
    expect(set.frames[199].endTime).toBe(43800);
    const [longer] = storyboardToPlyrThumbnails(HIGH, BASE, 50000);
    expect(longer.frames[199].endTime).toBe(50000);
  });

  it('stops at a sheet the manifest does not list instead of throwing', () => {
    const [set] = storyboardToPlyrThumbnails({ ...HIGH, images: HIGH.images.slice(0, 3) }, BASE, 43691);
    expect(set.frames).toHaveLength(150);
  });

  it('yields nothing for a degenerate variant', () => {
    expect(storyboardToPlyrThumbnails({ ...HIGH, rows: 0 }, BASE, 43691)).toEqual([]);
    expect(storyboardToPlyrThumbnails({ ...HIGH, interval_secs: 0 }, BASE, 43691)).toEqual([]);
  });
});
