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
  bar_name: 'Heist Train',
  bar_name_short: 'Heist',
  treasure_short: null,
  icon: null,
  trophy_icon: null,
  palette: { fill: ['#445ed6', '#fc52ae', '#ed815f', '#fc52ae', '#445ed6'], accent: '#fc52ae', badge: '#fc52ae' },
  channel_color: null,
};
const plain: HypeTrainLookData = {
  ...heist,
  theme: null,
  name: 'Hype Train',
  short_name: '',
  bar_name: '',
  bar_name_short: '',
  kickoff: null,
  palette: null,
};

describe('hypeTrainLook', () => {
  it('reads the palette and the look name from Rust', () => {
    expect(trainPalette({ look: heist })?.accent).toBe('#fc52ae');
    expect(trainPalette({ look: plain })).toBeNull();
    expect(trainPalette({})).toBeNull();
    expect(lookName(heist)).toBe('heist');
    expect(lookName({ ...plain, kind: 'golden_kappa' })).toBe('golden_kappa');
    expect(lookName(undefined)).toBe('regular');
  });

  it('names every train but a plain regular one', () => {
    expect(namesTrain(heist)).toBe(true);
    expect(namesTrain(plain)).toBe(false);
    expect(namesTrain({ ...plain, shared: true, bar_name: 'Shared Hype Train' })).toBe(true);
    expect(namesTrain({ ...plain, kind: 'mythic', bar_name: 'Mythic Train' })).toBe(true);
  });

  it('puts the type in the fill and the accent, never over the card or its ink', () => {
    const style = paletteStyle(heist.palette) as Record<string, string>;
    expect(style['--hype-fill-gradient']).toBe('linear-gradient(90deg, #445ed6, #fc52ae, #ed815f, #fc52ae, #445ed6)');
    expect(style['--hype-accent']).toBe('#fc52ae');
    // The card keeps the theme's own words: no ink override.
    expect(style['--color-text-primary']).toBeUndefined();
    expect(paletteStyle(null)).toEqual({});
  });

  it('throws confetti in the palette, the rainbow without one', () => {
    expect(confettiColors(heist.palette)).toEqual(['#445ed6', '#fc52ae', '#ed815f']);
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
