import { describe, expect, it } from 'vitest';
import { isPortraitGrid, thumbFitFor } from './thumbFit';

describe('thumbFitFor', () => {
  it('leaves every landscape platform on the card it always had', () => {
    for (const p of ['twitch', 'kick', 'youtube'] as const) {
      expect(thumbFitFor(p, false)).toBe('cover');
      // Even inside a portrait grid, a landscape source is never squeezed.
      expect(thumbFitFor(p, true)).toBe('cover');
    }
  });

  it('gives a portrait platform its own shape when it has the grid to itself', () => {
    expect(thumbFitFor('tiktok', true)).toBe('portrait');
  });

  it('keeps the mixed grid on one row height', () => {
    // Portrait wells in a landscape grid would make each row as tall as its
    // tallest card; the picture is shown whole inside a landscape well instead.
    expect(thumbFitFor('tiktok', false)).toBe('pillar');
  });
});

describe('isPortraitGrid', () => {
  it('is only ever true for a single portrait-first platform', () => {
    expect(isPortraitGrid('tiktok')).toBe(true);
    expect(isPortraitGrid('twitch')).toBe(false);
    expect(isPortraitGrid('all')).toBe(false);
    expect(isPortraitGrid(null)).toBe(false);
  });
});
