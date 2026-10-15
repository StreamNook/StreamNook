import { describe, expect, it } from 'vitest';
import { wornSubBadgeArt } from './wornSubBadge';

describe('wornSubBadgeArt', () => {
  it('picks the sub badge art from the badges chat hands over', () => {
    const art = wornSubBadgeArt([
      { key: 'moderator/1', info: { image_url_4x: 'https://cdn/mod.png' } },
      { key: 'subscriber/3012', info: { image_url_1x: 'https://cdn/sub1.png', image_url_4x: 'https://cdn/sub4.png' } },
    ]);
    expect(art).toBe('https://cdn/sub4.png');
  });

  it('reads a founder badge the same way', () => {
    expect(wornSubBadgeArt([{ key: 'founder/0', info: { image_url_2x: 'https://cdn/f.png' } }])).toBe('https://cdn/f.png');
  });

  it('is null without a sub badge or its art', () => {
    expect(wornSubBadgeArt(undefined)).toBeNull();
    expect(wornSubBadgeArt([{ key: 'vip/1', info: { image_url_4x: 'https://cdn/vip.png' } }])).toBeNull();
    expect(wornSubBadgeArt([{ key: 'subscriber/0', info: null }])).toBeNull();
  });
});
