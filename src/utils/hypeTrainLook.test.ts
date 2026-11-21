import { describe, expect, it } from 'vitest';
import type { HypeTrainLookData } from '../types';
import { badgeLook, confettiColors, justStarted, lookName, namesTrain, paletteStyle, trainPalette } from './hypeTrainLook';

const heist: HypeTrainLookData = {
  kind: 'regular',
  theme: 'heist',
  shared: false,
  all_time_high: false,
  treasure: null,
  name: 'Heist Train',
  short_name: 'Heist',
  kickoff: 'The Heist Is On!',
  treasure_words: null,
  bar_name: 'Heist',
  treasure_short: null,
  palette: { background: ['#445ed6', '#fc52ae', '#ed815f'], accent: '#ffd24a', fill: '#ffffff', badge: '#fc52ae' },
  channel_color: null,
};
const plain: HypeTrainLookData = { ...heist, theme: null, name: 'Hype Train', short_name: '', bar_name: '', kickoff: null, palette: null };

describe('hypeTrainLook', () => {
  it('reads the palette and the look name from Rust', () => {
    expect(trainPalette({ look: heist })?.accent).toBe('#ffd24a');
    expect(trainPalette({ look: plain })).toBeNull();
    expect(trainPalette({})).toBeNull();
    expect(lookName(heist)).toBe('heist');
    expect(lookName({ ...plain, kind: 'golden_kappa' })).toBe('golden_kappa');
    expect(lookName(undefined)).toBe('regular');
  });

  it('names every train but a plain regular one', () => {
    expect(namesTrain(heist)).toBe(true);
    expect(namesTrain(plain)).toBe(false);
    expect(namesTrain({ ...plain, shared: true, bar_name: 'Shared' })).toBe(true);
    expect(namesTrain({ ...plain, kind: 'mythic', bar_name: 'Mythic' })).toBe(true);
  });

  it('paints a palette as CSS variables with white text', () => {
    const style = paletteStyle(heist.palette) as Record<string, string>;
    expect(style['--hype-bg']).toBe('linear-gradient(90deg, #445ed6, #fc52ae, #ed815f)');
    expect(style['--hype-fill']).toBe('#ffffff');
    expect(style['--color-text-primary']).toBe('#ffffff');
    expect(paletteStyle(null)).toEqual({});
  });

  it('throws confetti in the palette, the rainbow without one', () => {
    expect(confettiColors(heist.palette)).toEqual(['#445ed6', '#fc52ae', '#ed815f', '#ffd24a', '#ffffff']);
    expect(confettiColors(null)).toEqual([]);
  });

  it('gives the card chip its color and word', () => {
    expect(badgeLook(heist, 5)).toEqual({ color: '#fc52ae', word: 'HEIST 5' });
    expect(badgeLook(plain, 3)).toEqual({ color: null, word: 'LVL 3' });
    expect(badgeLook(undefined, 2)).toEqual({ color: null, word: 'LVL 2' });
  });

  it('plays the kickoff only as the train begins', () => {
    const now = Date.parse('2026-11-21T12:00:10Z');
    expect(justStarted('2026-11-21T12:00:00Z', now)).toBe(true);
    expect(justStarted('2026-11-21T11:59:00Z', now)).toBe(false);
    expect(justStarted(null, now)).toBe(false);
  });
});
