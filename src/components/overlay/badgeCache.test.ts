import { describe, expect, it } from 'vitest';
import { cachedBadgeUrl, hiResBadgeUrl } from './overlayLayout';

const TWITCH_MOD = 'https://static-cdn.jtvnw.net/badges/v1/3267646d-33f0-4b17-b3df-f923a41db1d0/3';

describe('cachedBadgeUrl', () => {
  it('sends Twitch and add-on badge art through the site cache', () => {
    expect(cachedBadgeUrl(TWITCH_MOD)).toBe(
      `https://streamnook.app/api/v1/badge-art?src=${encodeURIComponent(TWITCH_MOD)}`,
    );
    expect(cachedBadgeUrl('https://cdn.frankerfacez.com/badge/2/4/rounded')).toContain('/api/v1/badge-art?src=');
  });

  it('sends 7TV badges through the 7TV badge route', () => {
    expect(cachedBadgeUrl('https://cdn.7tv.app/badge/01JZXYK6Q8S1Z6G3R4N9V7B2CD/4x.webp')).toBe(
      'https://streamnook.app/api/v1/7tv/badge/01JZXYK6Q8S1Z6G3R4N9V7B2CD/4x.webp',
    );
  });

  it('leaves addresses the cache does not take to load from their host', () => {
    expect(cachedBadgeUrl(undefined)).toBeNull();
    expect(cachedBadgeUrl('data:image/svg+xml,<svg/>')).toBeNull();
    expect(cachedBadgeUrl('https://example.com/badge.png')).toBeNull();
    expect(cachedBadgeUrl(`${TWITCH_MOD}?x=1`)).toBeNull();
    expect(cachedBadgeUrl('http://static-cdn.jtvnw.net/badges/v1/3267646d-33f0-4b17-b3df-f923a41db1d0/3')).toBeNull();
    // A 7TV id that is not a ULID never reaches the 7TV route.
    expect(cachedBadgeUrl('https://cdn.7tv.app/badge/60ae3e98b2ecb0150535c6b7/4x.webp')).toBeNull();
  });

  it('caches the largest art, the first address the badge asks for', () => {
    const small = 'https://static-cdn.jtvnw.net/badges/v1/3267646d-33f0-4b17-b3df-f923a41db1d0/1';
    expect(cachedBadgeUrl(hiResBadgeUrl(small))).toBe(cachedBadgeUrl(TWITCH_MOD));
  });
});
