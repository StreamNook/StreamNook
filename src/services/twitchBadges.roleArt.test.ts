// Run with: npm test
//
// A channel's FFZ moderator / VIP art reaches chat rows through the CHANNEL
// badge index this module keeps per room. Rust (services/ffz_room_badges.rs)
// substitutes the art into `fetch_channel_badges` and announces
// `twitch-channel-badges-changed` when it lands late or the setting flips.
// These tests hold the page half of that seam: an announce re-reads exactly
// the rooms this window has indexed, and rows parsed afterwards wear the new
// art while the role (set id, version, title) stays Twitch's.

import { beforeEach, describe, expect, it, vi } from 'vitest';

type Handler = (event: { payload: { channel_id?: string | null } }) => void;

const state = vi.hoisted(() => ({
  handlers: [] as Handler[],
  listenCalls: 0,
  channelSets: new Map<string, unknown>(),
  fetches: [] as string[],
  /** A room whose next read is held open until the test releases it. */
  held: new Map<string, Promise<void>>(),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, handler: Handler) => {
    state.listenCalls += 1;
    if (name === 'twitch-channel-badges-changed') state.handlers.push(handler);
    return () => {};
  }),
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (cmd: string, args?: { channelId?: string }) => {
    if (cmd === 'get_cached_global_badges') return GLOBAL;
    if (cmd === 'fetch_channel_badges') {
      const id = args?.channelId ?? '';
      state.fetches.push(id);
      // The answer is what Rust held when the read was made, not when it lands.
      const answer = state.channelSets.get(id) ?? { data: [] };
      const hold = state.held.get(id);
      if (hold) {
        state.held.delete(id);
        await hold;
      }
      return answer;
    }
    return undefined;
  }),
}));

const TWITCH_MOD = 'https://static-cdn.jtvnw.net/badges/v1/3267646d-33f0-4b17-b3df-f923a41db1d0/3';
const FFZ_MOD = 'data:image/svg+xml;base64,AAAA';
const FFZ_VIP = 'https://cdn.frankerfacez.com/room-badge/vip/id/23161357/v/e629b8b0/4';

const version = (url: string, title: string) => ({
  id: '1',
  image_url_1x: url,
  image_url_2x: url,
  image_url_4x: url,
  title,
  description: title,
  click_action: null,
  click_url: null,
});

const GLOBAL = {
  data: [
    { set_id: 'moderator', versions: [version(TWITCH_MOD, 'Moderator')] },
    { set_id: 'vip', versions: [version('https://static-cdn.jtvnw.net/badges/v1/vip/3', 'VIP')] },
  ],
};

const plainChannel = { data: [{ set_id: 'subscriber', versions: [version('sub.png', 'Subscriber')] }] };
const withRoleArt = {
  data: [
    ...plainChannel.data,
    { set_id: 'moderator', versions: [version(FFZ_MOD, 'Moderator')] },
    { set_id: 'vip', versions: [version(FFZ_VIP, 'VIP')] },
  ],
};

const flush = async () => {
  for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 5));
};

const { initializeBadgeCache, parseBadges } = await import('./twitchBadges');

const announce = async (channel_id: string | null) => {
  for (const h of state.handlers) h({ payload: { channel_id } });
  await flush();
};

describe('channel role art on the page', () => {
  beforeEach(() => {
    state.fetches.length = 0;
  });

  it('a row resolves moderator art from the channel set before the global one', async () => {
    state.channelSets.set('23161357', withRoleArt);
    await initializeBadgeCache('23161357');
    const [mod] = parseBadges('moderator/1', '23161357');
    expect(mod.key).toBe('moderator/1');
    expect(mod.info.title).toBe('Moderator');
    expect(mod.info.image_url_4x).toBe(FFZ_MOD);
    // Another room without art keeps Twitch's.
    expect(parseBadges('moderator/1', '71092938')[0].info.image_url_4x).toBe(TWITCH_MOD);
  });

  it('listens once per window however many rooms are indexed', async () => {
    await initializeBadgeCache('1');
    await initializeBadgeCache('2');
    await flush();
    expect(state.handlers).toHaveLength(1);
  });

  it('an announce for an indexed room re-reads it and new rows wear the new art', async () => {
    state.channelSets.set('500', plainChannel);
    await initializeBadgeCache('500');
    expect(parseBadges('vip/1', '500')[0].info.image_url_4x).not.toBe(FFZ_VIP);

    // The room's art lands after the first read (the 4 s wait ran out).
    state.channelSets.set('500', withRoleArt);
    state.fetches.length = 0;
    await announce('500');
    await vi.waitFor(() => {
      expect(state.fetches).toEqual(['500']);
      const [vip] = parseBadges('vip/1', '500');
      expect(vip.info.image_url_4x).toBe(FFZ_VIP);
      expect(vip.info.title).toBe('VIP');
    });
  });

  it('an announce for a room this window never indexed fetches nothing', async () => {
    await announce('999999');
    expect(state.fetches).toEqual([]);
  });

  it('art announced while the first read is still out is read again once it lands', async () => {
    // Rust answers the first read without art (the room was slow), then
    // announces the art before the page has indexed the room at all.
    state.channelSets.set('777', plainChannel);
    let release!: () => void;
    state.held.set('777', new Promise<void>((r) => (release = r)));
    const loading = initializeBadgeCache('777');
    await flush();
    state.channelSets.set('777', withRoleArt);
    await announce('777');
    expect(state.fetches.filter((id) => id === '777')).toHaveLength(1);
    release();
    await loading;
    // The re-read runs on its own turn after the first read lands; under a
    // loaded test run a fixed flush can end before it does, so wait for it.
    await vi.waitFor(() => {
      expect(state.fetches.filter((id) => id === '777')).toHaveLength(2);
      expect(parseBadges('moderator/1', '777')[0].info.image_url_4x).toBe(FFZ_MOD);
    });
  });

  it('a null channel (the setting flipped) re-reads every indexed room', async () => {
    state.channelSets.set('23161357', plainChannel);
    await announce(null);
    // Each re-read starts behind a dynamic import, so wait for them to land.
    await vi.waitFor(() => {
      expect(state.fetches).toContain('23161357');
      expect(state.fetches).toContain('500');
      // The setting is off now: Rust answers without art and rows go back to Twitch's.
      expect(parseBadges('moderator/1', '23161357')[0].info.image_url_4x).toBe(TWITCH_MOD);
    });
    expect(state.fetches).not.toContain('999999');
  });
});
