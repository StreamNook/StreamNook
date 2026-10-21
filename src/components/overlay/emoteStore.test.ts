// @vitest-environment node
// Run with: npm test
//
// Emotes through StreamNook's image store: which addresses go there, the order
// an emote tries its addresses in, and that the store is keyed by id.

import { describe, expect, it } from 'vitest';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { cachedEmoteUrl, emoteCandidates } from './overlayLayout';
import { OverlayChat } from './OverlayChat';
import { DEFAULT_OVERLAY_STYLE } from './overlayConfig';
import type { OverlayMessage } from './sampleMessages';

const STV = '01F6MQ33FG000FFJ97ZB8MWV52';
const STORE = 'https://streamnook.app/api/v1/emote-art';

describe('cachedEmoteUrl', () => {
  it('sends each CDN emote address to the store by provider, id and size', () => {
    expect(cachedEmoteUrl(`https://cdn.7tv.app/emote/${STV}/3x.webp`)).toBe(`${STORE}/7tv/${STV}/3x.webp`);
    expect(cachedEmoteUrl(`https://cdn.7tv.app/emote/${STV}/4x.avif`)).toBe(`${STORE}/7tv/${STV}/4x.avif`);
    expect(cachedEmoteUrl('https://cdn.betterttv.net/emote/56e9f494fff3cc5c35e5287e/3x')).toBe(
      `${STORE}/bttv/56e9f494fff3cc5c35e5287e/3x`,
    );
    expect(cachedEmoteUrl('https://cdn.betterttv.net/emote/56e9f494fff3cc5c35e5287e/2x.webp')).toBe(
      `${STORE}/bttv/56e9f494fff3cc5c35e5287e/2x.webp`,
    );
    expect(cachedEmoteUrl('//cdn.frankerfacez.com/emote/128054/4')).toBe(`${STORE}/ffz/128054/4`);
    expect(cachedEmoteUrl('https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/3.0')).toBe(
      `${STORE}/twitch/25/default/dark/3.0`,
    );
    expect(
      cachedEmoteUrl('https://static-cdn.jtvnw.net/emoticons/v2/emotesv2_dcd06b30a5c24f6eb871e8f5edbd44f7/animated/dark/4.0'),
    ).toBe(`${STORE}/twitch/emotesv2_dcd06b30a5c24f6eb871e8f5edbd44f7/animated/dark/4.0`);
  });

  it('leaves every other address to load from its CDN', () => {
    for (const url of [
      undefined,
      '',
      'https://example.com/emote.webp',
      `http://cdn.7tv.app/emote/${STV}/3x.webp`,
      `https://cdn.7tv.app/emote/${STV}/3x.webp?x=1`,
      `https://cdn.7tv.app/emote/${STV}/5x.webp`,
      `https://cdn.7tv.app/emote/${STV.toLowerCase()}/3x.webp`,
      'https://cdn.7tv.app/emote/60ae3e98b2ecb0150535c6b7/3x.webp',
      'https://cdn.betterttv.net/emote/56e9f494fff3cc5c35e5287e/4x',
      'https://cdn.frankerfacez.com/emote/128054/3',
      'https://static-cdn.jtvnw.net/emoticons/v2/25/default/light/3.0',
      'https://static-cdn.jtvnw.net/emoticons/v2/../../badges/x/default/dark/3.0',
      'https://cdn.7tv.app/badge/01JZXYK6Q8S1Z6G3R4N9V7B2CD/4x.webp',
    ]) {
      expect(cachedEmoteUrl(url), String(url)).toBeNull();
    }
  });
});

describe('emoteCandidates', () => {
  it('asks the store, then the CDN, then the id-rebuilt address (store first)', () => {
    const baked = `https://cdn.7tv.app/emote/${STV}/3x.avif`;
    expect(emoteCandidates({ emote_url: baked, emote_id: STV })).toEqual([
      `${STORE}/7tv/${STV}/3x.webp`,
      `https://cdn.7tv.app/emote/${STV}/3x.webp`,
    ]);
    // A wrong baked address still reaches the emote by its id.
    expect(emoteCandidates({ emote_url: 'https://example.com/broken.png', emote_id: '25' })).toEqual([
      'https://example.com/broken.png',
      `${STORE}/twitch/25/default/dark/3.0`,
      'https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/3.0',
    ]);
    // An address the store does not take loads straight from its host.
    expect(emoteCandidates({ emote_url: 'https://example.com/e.png' })).toEqual(['https://example.com/e.png']);
    expect(emoteCandidates({ emote_url: '' })).toEqual([]);
  });

  it('gives a gigantified emote its 4x art first, then the stored inline art', () => {
    expect(emoteCandidates({ emote_url: `https://cdn.7tv.app/emote/${STV}/3x.webp`, emote_id: STV }, true)).toEqual([
      `${STORE}/7tv/${STV}/4x.webp`,
      `${STORE}/7tv/${STV}/3x.webp`,
      `https://cdn.7tv.app/emote/${STV}/4x.webp`,
      `https://cdn.7tv.app/emote/${STV}/3x.webp`,
    ]);
  });

  it('keys the store by id: a renamed emote asks the same file', () => {
    const url = `https://cdn.7tv.app/emote/${STV}/3x.webp`;
    const before = emoteCandidates({ emote_url: url, emote_id: STV });
    const after = emoteCandidates({ emote_url: url, emote_id: STV });
    expect(after).toEqual(before);
    // And two emotes sharing a name (a seasonal set reusing it) never share a file.
    const other = '01GB9W8JN80004CKF2H1TWA99H';
    expect(emoteCandidates({ emote_url: `https://cdn.7tv.app/emote/${other}/3x.webp`, emote_id: other })[0]).toBe(
      `${STORE}/7tv/${other}/3x.webp`,
    );
  });
});

describe('overlay emote rows', () => {
  const row = (content: string): OverlayMessage => ({
    id: `m-${content}`,
    user_id: '1',
    username: 'viewer',
    display_name: 'viewer',
    color: '#ffffff',
    badges: [],
    timestamp: new Date(0).toISOString(),
    content,
    provider: 'twitch',
    channel: 'harborlight',
    emotes: [],
    layout: { height: 0, width: 0 },
    tags: {},
    segments: [{ type: 'emote', content, emote_id: STV, emote_url: `https://cdn.7tv.app/emote/${STV}/3x.webp` }],
  });

  it('draws an emote from the store by its id, whatever its name is now', () => {
    for (const name of ['OldName', 'NewName']) {
      const html = renderToStaticMarkup(createElement(OverlayChat, { messages: [row(name)], style: DEFAULT_OVERLAY_STYLE }));
      expect(html).toContain(`src="${STORE}/7tv/${STV}/3x.webp"`);
      expect(html).toContain(`alt="${name}"`);
    }
  });
});
