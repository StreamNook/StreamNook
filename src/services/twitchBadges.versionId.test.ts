// Run with: npm test
//
// A chat row's Twitch badge must carry its VERSION id: the Twitch badge page
// (BadgeDetailOverlay) keys fetch_badge_metadata, get_badge_earn_guide,
// get_badge_window and get_badge_drop_campaign on it. The index used to keep
// only the art and title, so a badge opened from chat asked for `undefined`
// and never showed its metadata, earn guide or window.
import { describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));

const version = (id: string, title: string) => ({
  id,
  image_url_1x: `https://static-cdn.jtvnw.net/badges/v1/${title}/1`,
  image_url_2x: `https://static-cdn.jtvnw.net/badges/v1/${title}/2`,
  image_url_4x: `https://static-cdn.jtvnw.net/badges/v1/${title}/3`,
  title,
  description: title,
  click_action: null,
  click_url: null,
});

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (cmd: string) => {
    if (cmd === 'get_cached_global_badges') {
      return {
        data: [
          { set_id: 'premium', versions: [version('1', 'Prime Gaming')] },
          { set_id: 'bits', versions: [version('100', 'cheer 100'), version('1000', 'cheer 1000')] },
        ],
      };
    }
    if (cmd === 'fetch_channel_badges') {
      return { data: [{ set_id: 'subscriber', versions: [version('3012', 'Subscriber 1-Year')] }] };
    }
    return undefined;
  }),
}));

import { initializeBadgeCache, parseBadges } from './twitchBadges';
import { badgeDetailsRoute, twitchRowBadgeRef } from '../utils/badgeSpotlight';

describe('the Twitch badge index carries version ids', () => {
  it('global and channel badges both keep their version id', async () => {
    await initializeBadgeCache('room-1');
    const [prime, bits, sub] = parseBadges('premium/1,bits/1000,subscriber/3012', 'room-1');
    expect(prime.info.id).toBe('1');
    expect(bits.info.id).toBe('1000');
    expect(sub.info.id).toBe('3012');
  });

  it('the id reaches the Twitch badge page route a click opens', async () => {
    await initializeBadgeCache('room-1');
    const [sub] = parseBadges('subscriber/3012', 'room-1');
    const route = badgeDetailsRoute(twitchRowBadgeRef(sub.key, sub.info));
    expect(route).toMatchObject({ kind: 'twitch', setId: 'subscriber', badge: { id: '3012', title: 'Subscriber 1-Year' } });
  });
});
