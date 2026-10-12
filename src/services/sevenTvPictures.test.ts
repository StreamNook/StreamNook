import { describe, expect, it } from 'vitest';
import { isAnimatedPicture, pickSevenTvPictureUrl, sevenTvIdForStream, type SevenTvPicture } from './sevenTvPictures';

// The shapes Rust emits for two real 7TV pictures (seventv_cosmetics_resolver.rs
// fixtures): an animated upload with `_static` first frames, and a still one.
const base = 'https://cdn.7tv.app/user/u/profile-picture/p';
const scales = [1, 2, 3, 4];
const animated: SevenTvPicture = {
  id: 'p',
  still: scales.map((scale) => ({ scale, url: `${base}/${scale}x_static.webp` })),
  animated: scales.map((scale) => ({ scale, url: `${base}/${scale}x.webp` })),
};
const still: SevenTvPicture = {
  id: 's',
  still: scales.map((scale) => ({ scale, url: `${base}/${scale}x.webp` })),
  animated: [],
};

describe('sevenTvIdForStream', () => {
  it('keys each platform the way 7TV knows it, and never crosses platforms', () => {
    expect(sevenTvIdForStream({ user_id: '71092938' })).toBe('71092938');
    expect(sevenTvIdForStream({ provider: 'twitch', user_id: '71092938' })).toBe('71092938');
    // A bare Kick number would be read as a Twitch id; it must carry its platform.
    expect(sevenTvIdForStream({ provider: 'kick', user_id: '26298' })).toBe('kick:26298');
    expect(sevenTvIdForStream({ provider: 'youtube', user_id: 'UCX6OQ3DkcsbYNE6H8uQQuVA' })).toBe(
      'youtube:UCX6OQ3DkcsbYNE6H8uQQuVA',
    );
  });

  it('asks nothing for ids 7TV cannot know', () => {
    expect(sevenTvIdForStream({ provider: 'tiktok', user_id: '123' })).toBeNull();
    expect(sevenTvIdForStream({ provider: 'youtube', user_id: 'dQw4w9WgXcQ' })).toBeNull();
    expect(sevenTvIdForStream({ provider: 'kick', user_id: 'xqc' })).toBeNull();
    expect(sevenTvIdForStream({ user_id: '' })).toBeNull();
  });
});

describe('pickSevenTvPictureUrl', () => {
  it('animates only where the surface may move', () => {
    // Profile card: 80 px at 1x needs 3x (96 px), moving.
    expect(pickSevenTvPictureUrl(animated, { px: 80, animate: true })).toBe(`${base}/3x.webp`);
    // Sidebar row at rest: the first frame.
    expect(pickSevenTvPictureUrl(animated, { px: 32, animate: false })).toBe(`${base}/1x_static.webp`);
  });

  it('picks the smallest rendition that stays sharp at the pixel ratio', () => {
    expect(pickSevenTvPictureUrl(animated, { px: 32, animate: false, dpr: 2 })).toBe(`${base}/2x_static.webp`);
    expect(pickSevenTvPictureUrl(animated, { px: 20, animate: true, dpr: 1.5 })).toBe(`${base}/1x.webp`);
    // Bigger than 4x: the largest there is.
    expect(pickSevenTvPictureUrl(animated, { px: 200, animate: true })).toBe(`${base}/4x.webp`);
  });

  it('draws a still picture whether or not the surface may move', () => {
    expect(pickSevenTvPictureUrl(still, { px: 32, animate: true })).toBe(`${base}/1x.webp`);
    expect(isAnimatedPicture(still)).toBe(false);
    expect(isAnimatedPicture(animated)).toBe(true);
  });

  it('falls back (null) when there is nothing to draw', () => {
    expect(pickSevenTvPictureUrl(null, { px: 32, animate: true })).toBeNull();
    // A moving picture without first frames never animates in a still slot.
    const noStill: SevenTvPicture = { ...animated, still: [] };
    expect(pickSevenTvPictureUrl(noStill, { px: 32, animate: false })).toBeNull();
    expect(pickSevenTvPictureUrl(noStill, { px: 32, animate: true })).toBe(`${base}/1x.webp`);
  });

  it('handles a missing scale by taking the next one up', () => {
    const gappy: SevenTvPicture = { id: 'g', still: [{ scale: 1, url: 'a' }, { scale: 3, url: 'c' }], animated: [] };
    expect(pickSevenTvPictureUrl(gappy, { px: 48, animate: false })).toBe('c');
  });
});
