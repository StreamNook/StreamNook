// Run with: npm test
//
// A channel-points redemption row (built in Rust, `redemption_message` in
// irc_service.rs) and every other event row must draw a chatter's subscriber
// badge from the CHANNEL's set, exactly like a normal chat row. The badge
// cache looks a badge up in the set of the room the row names; a row that
// names none falls through to Twitch's global set, whose subscriber badge is
// the default blue star. These tests run the real badge cache (Rust answers
// mocked) through the in-app chat row's parse and the preview's resolver.

import { beforeAll, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (cmd: string, args?: { channelId?: string }) => {
    if (cmd === 'get_cached_global_badges') return GLOBAL;
    if (cmd === 'fetch_channel_badges') return CHANNEL_SETS[args?.channelId ?? ''] ?? { data: [] };
    return undefined;
  }),
}));

import { initializeBadgeCache, parseBadges } from '../../services/twitchBadges';
import { parseMessage, type BackendChatMessage } from '../../services/twitchChat';
import { badgeRoomOf, overlayTwitchBadges } from './liveOverlayRows';

const ROOM = '134522528';
const PARTNER = '22484632';
const DEFAULT_STAR = 'https://static-cdn.jtvnw.net/badges/v1/e8984705-d091-4e54-8241-e53b30a84b0e/3';
const CHANNEL_SUN = 'https://static-cdn.jtvnw.net/badges/v1/a5d92e36-e94a-4ea5-b17e-40bdb0a34bc3/3';
const PARTNER_ART = 'https://static-cdn.jtvnw.net/badges/v1/partner-sub-3/3';

const version = (id: string, url: string, title: string) => ({
  id,
  image_url_1x: url,
  image_url_2x: url,
  image_url_4x: url,
  title,
  description: title,
});

const GLOBAL = {
  data: [{ set_id: 'subscriber', versions: [version('3', DEFAULT_STAR, 'Subscriber')] }],
};
const CHANNEL_SETS: Record<string, unknown> = {
  [ROOM]: { data: [{ set_id: 'subscriber', versions: [version('3', CHANNEL_SUN, '3-Month Subscriber')] }] },
  [PARTNER]: { data: [{ set_id: 'subscriber', versions: [version('3', PARTNER_ART, '3-Month Subscriber')] }] },
};

const SUB = [{ name: 'subscriber', version: '3' }];

/** The tags Rust puts on a redemption row (`redemption_message`). */
const redemptionTags = (roomId?: string): Record<string, string> => ({
  'user-id': '826207736',
  ...(roomId ? { 'room-id': roomId } : {}),
  id: 'redeem-r-1',
  'display-name': 'SketchyRandomGuy',
  'custom-reward-id': 'reward-9',
  'sn-reward-title': 'Hydrate',
  'sn-reward-cost': '500',
});

const subUrl = (tags: Record<string, string>, fallbackRoomId?: string | null): string | undefined =>
  overlayTwitchBadges(SUB, tags, parseBadges, undefined, fallbackRoomId)?.find((b) => b.name === 'subscriber')
    ?.image_url_4x;

beforeAll(async () => {
  await initializeBadgeCache(ROOM);
  await initializeBadgeCache(PARTNER);
});

describe('event rows draw the channel sub badge', () => {
  it('a redemption row naming its channel draws the channel art', () => {
    expect(subUrl(redemptionTags(ROOM))).toBe(CHANNEL_SUN);
  });

  it('a redemption row with no room id draws the default star (the reported bug)', () => {
    expect(subUrl(redemptionTags())).toBe(DEFAULT_STAR);
  });

  it('a row without a room id takes the source channel the preview knows', () => {
    expect(subUrl(redemptionTags(), ROOM)).toBe(CHANNEL_SUN);
    // The row's own room id wins over the source's.
    expect(subUrl(redemptionTags(PARTNER), ROOM)).toBe(PARTNER_ART);
  });

  it('a shared-chat row wears the art of the channel it was sent in', () => {
    const tags = { 'room-id': ROOM, 'source-room-id': PARTNER, 'user-id': '1' };
    expect(badgeRoomOf(tags)).toBe(PARTNER);
    expect(subUrl(tags)).toBe(PARTNER_ART);
  });

  it.each([
    ['a normal chat row', { 'room-id': ROOM, 'user-id': '1' }],
    ['a sub notice', { 'room-id': ROOM, 'msg-id': 'resub', 'user-id': '1' }],
    ['a gift', { 'room-id': ROOM, 'msg-id': 'subgift', 'user-id': '1' }],
    ['a cheer', { 'room-id': ROOM, bits: '100', 'user-id': '1' }],
    ['a highlighted message', { 'room-id': ROOM, 'msg-id': 'highlighted-message', 'user-id': '1' }],
    ['a first message', { 'room-id': ROOM, 'first-msg': '1', 'user-id': '1' }],
    ['a text reward', { 'room-id': ROOM, 'custom-reward-id': 'reward-2', 'user-id': '1' }],
  ])('%s draws the channel art', (_label, tags) => {
    expect(subUrl(tags)).toBe(CHANNEL_SUN);
  });

  it('no badges resolve to nothing, so the row keeps its own', () => {
    expect(overlayTwitchBadges([], redemptionTags(ROOM), parseBadges, undefined)).toBeNull();
  });
});

/** A redemption row as Rust sends it over the bridge (`redemption_message`). */
const rustRedemptionRow = (roomId?: string): BackendChatMessage =>
  ({
    id: 'redeem-r-1',
    user_id: '826207736',
    username: 'sketchyrandomguy',
    display_name: 'SketchyRandomGuy',
    color: '#FFFFFF',
    badges: [
      { name: 'subscriber', version: '3' },
      { name: '2026-worlds-costumed-pikachu', version: '1' },
    ],
    timestamp: '1000',
    content: 'Hydrate',
    provider: 'twitch',
    channel: 'feelssunnyman',
    emotes: [],
    tags: redemptionTags(roomId),
    segments: [{ type: 'text', content: 'Hydrate' }],
  }) as unknown as BackendChatMessage;

/** The chat row's own channel pick (ChatMessage: source-room-id, else room-id). */
const chatRowSubUrl = (row: BackendChatMessage): string | undefined => {
  const channelId = row.tags?.['source-room-id'] || row.tags?.['room-id'];
  const parsed = parseMessage(row, channelId);
  const sub = parsed.badges.find((b) => b.key === 'subscriber/3');
  return sub?.info?.image_url_4x;
};

describe('in-app chat redemption row', () => {
  it('draws the channel sub art once Rust names the channel', () => {
    expect(chatRowSubUrl(rustRedemptionRow(ROOM))).toBe(CHANNEL_SUN);
  });

  it('drew the default star while the row carried no room id', () => {
    expect(chatRowSubUrl(rustRedemptionRow())).toBe(DEFAULT_STAR);
  });
});
