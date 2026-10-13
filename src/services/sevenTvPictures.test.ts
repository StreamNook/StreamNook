import { describe, expect, it } from 'vitest';
import {
  DEFAULT_PROFILE_PICTURE_ANIMATION,
  DEFAULT_PROFILE_PICTURE_SOURCE,
  isAnimatedPicture,
  isPictureRendition,
  isSevenTvCosmeticsId,
  profilePictureSource,
  sevenTvLookupId,
  pickSevenTvPictureUrl,
  pictureMovesNow,
  picturePlan,
  sevenTvIdForStream,
  type SevenTvMotion,
  type SevenTvPicture,
} from './sevenTvPictures';

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

describe('profilePictureSource', () => {
  it('defaults to 7TV, as Rust does, and reads only "twitch" as the platform source', () => {
    expect(DEFAULT_PROFILE_PICTURE_SOURCE).toBe('seventv');
    expect(profilePictureSource(undefined)).toBe('seventv');
    expect(profilePictureSource(null)).toBe('seventv');
    expect(profilePictureSource('seventv')).toBe('seventv');
    expect(profilePictureSource('anything else')).toBe('seventv');
    expect(profilePictureSource('twitch')).toBe('twitch');
  });
});

describe('isSevenTvCosmeticsId', () => {
  // Same table as Rust's only_ids_7tv_can_know_are_looked_up.
  it('accepts what has_seventv_platform accepts', () => {
    expect(isSevenTvCosmeticsId('71092938')).toBe(true);
    expect(isSevenTvCosmeticsId('kick:26298')).toBe(true);
    expect(isSevenTvCosmeticsId('youtube:UCX6OQ3DkcsbYNE6H8uQQuVA')).toBe(true);
  });

  it('refuses logins, TikTok, a Kick slug and a YouTube video id', () => {
    expect(isSevenTvCosmeticsId('xqc')).toBe(false);
    expect(isSevenTvCosmeticsId('tiktok:123')).toBe(false);
    expect(isSevenTvCosmeticsId('kick:xqc')).toBe(false);
    expect(isSevenTvCosmeticsId('youtube:dQw4w9WgXcQ')).toBe(false);
    expect(isSevenTvCosmeticsId('')).toBe(false);
    expect(isSevenTvCosmeticsId('123456789012345678901')).toBe(false);
  });
});

describe('sevenTvLookupId', () => {
  it('looks nobody up on the Twitch source', () => {
    for (const id of ['71092938', 'kick:26298', 'youtube:UCX6OQ3DkcsbYNE6H8uQQuVA']) {
      expect(sevenTvLookupId('twitch', id)).toBeNull();
      expect(sevenTvLookupId('twitch', id, false)).toBeNull();
    }
  });

  it('looks up ids 7TV can know on the 7TV source, and nothing else', () => {
    expect(sevenTvLookupId('seventv', '71092938')).toBe('71092938');
    expect(sevenTvLookupId('seventv', 'kick:26298')).toBe('kick:26298');
    expect(sevenTvLookupId('seventv', 'tiktok:123')).toBeNull();
    expect(sevenTvLookupId('seventv', null)).toBeNull();
    expect(sevenTvLookupId('seventv', undefined)).toBeNull();
  });

  it('reads a primed id as it is when nothing is asked', () => {
    expect(sevenTvLookupId('seventv', 'streamnook:sample', false)).toBe('streamnook:sample');
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

describe('picturePlan', () => {
  const surfaces: SevenTvMotion[] = ['animate', 'hover', 'still'];

  it('defaults to Always, so an unset or unknown choice moves every surface', () => {
    expect(DEFAULT_PROFILE_PICTURE_ANIMATION).toBe('always');
    for (const surface of surfaces) {
      expect(picturePlan(undefined, surface)).toBe('moving');
      expect(picturePlan(null, surface)).toBe('moving');
      expect(picturePlan('sideways', surface)).toBe('moving');
      expect(picturePlan('always', surface)).toBe('moving');
    }
  });

  it('On hover lets the surface decide', () => {
    expect(picturePlan('hover', 'animate')).toBe('moving');
    expect(picturePlan('hover', 'hover')).toBe('hover');
    expect(picturePlan('hover', 'still')).toBe('still');
  });

  it('Never draws every surface still', () => {
    for (const surface of surfaces) expect(picturePlan('never', surface)).toBe('still');
  });
});

describe('pictureMovesNow', () => {
  const shown = { hovered: false, onScreen: true, windowHidden: false };

  it('plays a moving picture only while it is on screen in a shown window', () => {
    expect(pictureMovesNow('moving', shown)).toBe(true);
    expect(pictureMovesNow('moving', { ...shown, onScreen: false })).toBe(false);
    expect(pictureMovesNow('moving', { ...shown, windowHidden: true })).toBe(false);
  });

  it('plays a hover picture only under the pointer', () => {
    expect(pictureMovesNow('hover', shown)).toBe(false);
    expect(pictureMovesNow('hover', { ...shown, hovered: true })).toBe(true);
    expect(pictureMovesNow('hover', { ...shown, hovered: true, windowHidden: true })).toBe(false);
  });

  it('never plays a still picture', () => {
    expect(pictureMovesNow('still', { hovered: true, onScreen: true, windowHidden: false })).toBe(false);
  });
});

describe('isPictureRendition', () => {
  it('knows the still and moving renditions of one picture, and nothing else', () => {
    expect(isPictureRendition(animated, `${base}/2x_static.webp`)).toBe(true);
    expect(isPictureRendition(animated, `${base}/2x.webp`)).toBe(true);
    expect(isPictureRendition(animated, 'https://static-cdn.jtvnw.net/x.png')).toBe(false);
    expect(isPictureRendition(null, `${base}/2x.webp`)).toBe(false);
    expect(isPictureRendition(animated, null)).toBe(false);
  });
});
