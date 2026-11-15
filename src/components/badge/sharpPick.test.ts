// Run with: npm test
//
// Which sharp file a badge image draws at its size: Twitch's own image up to 72
// device px, the 144 px WebP up to 144, the 288 px master above, and never art
// made from a different image than the one drawn.

import { describe, expect, it } from 'vitest';
import { pickSharp } from './sharpPick';

const L = '34d2d3cad154dbffb1ce314ce9cbbaf767dccbfdce7c9acc739edb19eac42401';
const S = '9801d687fca7d8744e77290e289a35535eeea91bac87bf28e3039d580ea7d605';
const LARGE = `https://cdn.streamnook.app/twitch-badges/${L}.png`;
const SMALL = `https://cdn.streamnook.app/twitch-badges/${S}.webp`;
const TWITCH = 'https://static-cdn.jtvnw.net/badges/v1/abc/3';
const art = { i: TWITCH, a: `${L}.${S}` };

describe('pickSharp', () => {
  it('draws Twitch with no art', () => {
    expect(pickSharp(64, 2, TWITCH, null)).toBeNull();
    expect(pickSharp(64, 2, TWITCH, undefined)).toBeNull();
  });

  it('draws Twitch when the art was made from a different image', () => {
    expect(pickSharp(64, 2, 'https://static-cdn.jtvnw.net/badges/v1/other/3', art)).toBeNull();
  });

  it('draws Twitch when the base is malformed', () => {
    expect(pickSharp(64, 2, TWITCH, { i: TWITCH, a: L })).toBeNull();
  });

  it('picks by device px: up to 72 Twitch, up to 144 small, above large', () => {
    expect(pickSharp(72, 1, TWITCH, art)).toBeNull();
    expect(pickSharp(36, 2, TWITCH, art)).toBeNull();
    expect(pickSharp(73, 1, TWITCH, art)).toBe(SMALL);
    expect(pickSharp(40, 2, TWITCH, art)).toBe(SMALL);
    expect(pickSharp(72, 2, TWITCH, art)).toBe(SMALL);
    expect(pickSharp(48, 3, TWITCH, art)).toBe(SMALL);
    expect(pickSharp(80, 2, TWITCH, art)).toBe(LARGE);
    expect(pickSharp(64, 3, TWITCH, art)).toBe(LARGE);
  });

  it('reads an unusable ratio as 1x', () => {
    expect(pickSharp(64, 0, TWITCH, art)).toBeNull();
    expect(pickSharp(64, Number.NaN, TWITCH, art)).toBeNull();
  });
});
