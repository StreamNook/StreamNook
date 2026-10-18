import { beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
  convertFileSrc: (p: string) => `asset://${p}`,
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }));

import { canCredit, creatorRoleLabel, creditLines, emoteLargeUrl, emoteProviderLabel, fetchEmoteCredit } from './emoteCredit';
import { matchEmotes } from './emoteMatch';

describe('creditLines', () => {
  it('credits the Twitch artist and names the owning channel', () => {
    expect(
      creditLines('twitch', { provider: 'twitch', ownerName: 'pokimane', artistName: 'Seabii', kindLabel: 'Sub emote' }),
    ).toEqual(['Art by Seabii', 'pokimane · Sub emote']);
  });

  it('names the channel when Twitch records no artist', () => {
    expect(creditLines('twitch', { provider: 'twitch', ownerName: 'xqc', kindLabel: 'Sub emote' })).toEqual([
      'From xqc',
      'Twitch · Sub emote',
    ]);
  });

  it('a Twitch global has no owner', () => {
    expect(creditLines('twitch', { provider: 'twitch', kindLabel: 'Global emote' })).toEqual(['Twitch', 'Global emote']);
  });

  it('credits a 7TV uploader and an alias', () => {
    expect(
      creditLines('7tv', { provider: '7tv', ownerName: 'Alpha', originalName: 'xqcTours', kindLabel: 'Channel emote' }),
    ).toEqual(['by Alpha', 'Alias of xqcTours · 7TV']);
    expect(creditLines('7tv', { provider: '7tv', ownerName: 'Alpha', kindLabel: 'Channel emote' })).toEqual([
      'by Alpha',
      'Channel emote · 7TV',
    ]);
  });

  it('shows the FFZ artist and who added it', () => {
    expect(creditLines('ffz', { provider: 'ffz', ownerName: 'Own', artistName: 'Drawer' })).toEqual([
      'Art by Drawer',
      'Added by Own · FrankerFaceZ',
    ]);
  });

  it('falls back to the provider while nothing is known', () => {
    expect(creditLines('bttv', null)).toEqual(['BetterTTV', '']);
    expect(creditLines('kick', null)).toEqual(['Kick', '']);
    expect(emoteProviderLabel('nope')).toBe('Emote');
  });
});

describe('fetchEmoteCredit', () => {
  beforeEach(() => invoke.mockReset());

  it('asks Rust and maps the answer', async () => {
    invoke.mockResolvedValueOnce({ provider: 'twitch', owner_name: 'pokimane', artist_name: 'Seabii', kind_label: 'Sub emote' });
    const c = await fetchEmoteCredit({ provider: 'twitch', id: 'emotesv2_x', name: 'pokiYap', channel: 'pokimane', channelId: '44445592' });
    expect(invoke).toHaveBeenCalledWith('emote_credit', {
      provider: 'twitch',
      id: 'emotesv2_x',
      name: 'pokiYap',
      channel: 'pokimane',
      channelId: '44445592',
      detail: null,
    });
    expect(c).toEqual({ provider: 'twitch', ownerName: 'pokimane', artistName: 'Seabii', originalName: undefined, kindLabel: 'Sub emote' });
  });

  it('never asks for a provider it cannot credit', async () => {
    expect(canCredit('kick')).toBe(false);
    expect(await fetchEmoteCredit({ provider: 'kick', id: '1' })).toBeNull();
    expect(invoke).not.toHaveBeenCalled();
  });

  it('a failed lookup is no credit, not an error', async () => {
    invoke.mockRejectedValueOnce(new Error('denied'));
    expect(await fetchEmoteCredit({ provider: '7tv', id: '01G3WEGZN0000ET2J0MQP5YJ0G' })).toBeNull();
  });
});

describe('matchEmotes picker mode', () => {
  beforeEach(() => invoke.mockReset());

  it('passes the picker flag and returns per-provider counts and row details', async () => {
    invoke.mockResolvedValueOnce({
      rows: [
        {
          id: 'a',
          name: 'PogU',
          url: 'u',
          provider: '7tv',
          source_label: '7TV',
          source_detail: '',
          owner_name: 'Alpha',
          width: 64,
          favorite: true,
          locked: false,
        },
      ],
      total: 3,
      ready: true,
      counts: { '7tv': 2, bttv: 1 },
    });
    const res = await matchEmotes({ provider: 'twitch', channel: 'xqc', channelId: '71092938' }, 'pog', 'search', {
      limit: 200,
      picker: true,
    });
    expect(invoke).toHaveBeenCalledWith('match_emote_tokens', expect.objectContaining({ picker: true, limit: 200, query: 'pog' }));
    expect(res.counts).toEqual({ '7tv': 2, bttv: 1 });
    expect(res.rows[0]).toMatchObject({ ownerName: 'Alpha', width: 64, favorite: true });
  });
});

describe('emote card detail', () => {
  beforeEach(() => invoke.mockReset());

  it('asks for detail and maps creators with their links and the emote page', async () => {
    invoke.mockResolvedValueOnce({
      provider: '7tv',
      owner_name: 'petepogu',
      kind_label: 'Channel emote',
      creators: [
        {
          role: 'uploader',
          name: 'petepogu',
          avatar_url: 'https://cdn.7tv.app/a.avif',
          link: { kind: 'web', url: 'https://7tv.app/users/01FFFKKA2R0007P57XYW0BHEQV', site: '7TV' },
        },
      ],
      page_url: 'https://7tv.app/emotes/01G3WEGZN0000ET2J0MQP5YJ0G',
    });
    const c = await fetchEmoteCredit({ provider: '7tv', id: '01G3WEGZN0000ET2J0MQP5YJ0G', name: 'Special', detail: true });
    expect(invoke).toHaveBeenCalledWith('emote_credit', expect.objectContaining({ detail: true }));
    expect(c?.creators?.[0]).toEqual({
      role: 'uploader',
      name: 'petepogu',
      avatarUrl: 'https://cdn.7tv.app/a.avif',
      link: { kind: 'web', url: 'https://7tv.app/users/01FFFKKA2R0007P57XYW0BHEQV', site: '7TV' },
    });
    expect(c?.pageUrl).toBe('https://7tv.app/emotes/01G3WEGZN0000ET2J0MQP5YJ0G');
  });

  it('a hover never asks for detail', async () => {
    invoke.mockResolvedValueOnce({ provider: 'ffz' });
    await fetchEmoteCredit({ provider: 'ffz', id: '246878' });
    expect(invoke).toHaveBeenCalledWith('emote_credit', expect.objectContaining({ detail: null }));
  });

  it('labels each chip by role and picks the largest image per provider', () => {
    expect(creatorRoleLabel('artist')).toBe('Art by');
    expect(creatorRoleLabel('uploader')).toBe('Made by');
    expect(creatorRoleLabel('channel')).toBe('From the channel');
    expect(emoteLargeUrl('twitch', 'x', 'https://static-cdn.jtvnw.net/emoticons/v2/x/default/dark/1.0')).toBe(
      'https://static-cdn.jtvnw.net/emoticons/v2/x/default/dark/3.0',
    );
    expect(emoteLargeUrl('ffz', '246878', 'https://cdn.frankerfacez.com/emote/246878/1')).toBe('https://cdn.frankerfacez.com/emote/246878/4');
    expect(emoteLargeUrl('bttv', 'abc', 'u')).toBe('https://cdn.betterttv.net/emote/abc/3x');
    expect(emoteLargeUrl('7tv', 'id', 'u')).toBe('https://cdn.7tv.app/emote/id/4x.avif');
  });
});
