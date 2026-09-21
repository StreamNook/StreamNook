import { describe, expect, it } from 'vitest';
import { adjustNameColor, nameColorReadsLight } from './nameColor';

describe('adjustNameColor', () => {
  it('is a no-op when off', () => {
    expect(adjustNameColor('#000080', 'off', true)).toBe('#000080');
  });

  it('leaves non-hex values alone', () => {
    expect(adjustNameColor('', 'hsl_loop', true)).toBe('');
    expect(adjustNameColor('var(--color-accent)', 'hsl_loop', true)).toBe('var(--color-accent)');
    expect(adjustNameColor('rgb(1,2,3)', 'hsl_loop', true)).toBe('rgb(1,2,3)');
  });

  it('lifts a dark name until it reads light on a dark theme', () => {
    const out = adjustNameColor('#000080', 'hsl_loop', true);
    expect(out).not.toBe('#000080');
    expect(nameColorReadsLight(out)).toBe(true);
  });

  it('keeps a color that already reads light on a dark theme', () => {
    expect(adjustNameColor('#ff7f50', 'hsl_loop', true)).toBe('#ff7f50');
  });

  it('darkens a pale name on a light theme', () => {
    const out = adjustNameColor('#ffff99', 'hsl_loop', false);
    expect(nameColorReadsLight(out)).toBe(false);
  });

  it('preserves hue: an adjusted navy is still blue', () => {
    const out = adjustNameColor('#000080', 'hsl_loop', true);
    const r = parseInt(out.slice(1, 3), 16);
    const b = parseInt(out.slice(5, 7), 16);
    expect(b).toBeGreaterThan(r);
  });

  it('accepts three-digit hex', () => {
    expect(nameColorReadsLight(adjustNameColor('#008', 'hsl_loop', true))).toBe(true);
  });

  it('always terminates, even for black', () => {
    // Black has no hue and lightness 0; the walk still climbs.
    const out = adjustNameColor('#000000', 'hsl_loop', true);
    expect(nameColorReadsLight(out)).toBe(true);
  });
});
