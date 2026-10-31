import { describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

import cases from './emoteRefCases.json';
import { emoteClickRoute, emoteRef, stackTargets } from './emoteClick';

describe('which service an emote image comes from', () => {
  it('follows the cases Rust tests too', () => {
    for (const c of cases) expect(emoteRef(c.url, c.id), `${c.url} ${c.id}`).toBe(c.source);
  });
});

describe('where a desktop click goes', () => {
  it('opens the 7TV spotlight only where it is mounted, else the card, never for Kick', () => {
    expect(emoteClickRoute('7tv', true)).toBe('spotlight');
    expect(emoteClickRoute('7tv', false)).toBe('card');
    expect(emoteClickRoute('bttv', true)).toBe('card');
    expect(emoteClickRoute('kick', true)).toBe('none');
    expect(emoteClickRoute(null, true)).toBe('none');
  });
});

describe('what a phone tap on a stack opens', () => {
  const room = { login: 'xqc', id: '71092938' };
  const seventv = { emoteId: '01G3WEGZN0000ET2J0MQP5YJ0G', emoteUrl: 'https://cdn.7tv.app/emote/01G3WEGZN0000ET2J0MQP5YJ0G/1x.webp', content: 'RainTime' };
  const twitch = { emoteId: '25', emoteUrl: 'https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/1.0', content: 'Kappa' };
  const kick = { emoteId: '37226', emoteUrl: 'https://files.kick.com/emotes/37226/fullsize', content: 'kickEmote' };

  it('opens the base and lists every creditable member in order', () => {
    const t = stackTargets([twitch, seventv], room);
    expect(t.map((x) => x.provider)).toEqual(['twitch', '7tv']);
    expect(t[0]).toMatchObject({ id: '25', name: 'Kappa', channelLogin: 'xqc', channelId: '71092938' });
  });

  it('opens the first creditable member when the base is not', () => {
    expect(stackTargets([kick, seventv], room).map((x) => x.name)).toEqual(['RainTime']);
    expect(stackTargets([{ content: 'emoji' }, seventv], room).map((x) => x.name)).toEqual(['RainTime']);
  });

  it('opens nothing when nothing can be credited', () => {
    expect(stackTargets([kick], room)).toEqual([]);
  });

  it("fills a Twitch emote's image the way rows draw it", () => {
    expect(stackTargets([{ emoteId: '25', content: 'Kappa' }], null)[0].url).toBe(
      'https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/2.0',
    );
  });
});
