// Run with: npm test
//
// Text that reaches the page without Twitch's emote positions (a local echo
// before the server's copy, a reply preview, and plain text on Kick and
// YouTube) resolves names through these lookups. A Twitch emote must win over
// a third-party emote with the same name, as it does in Rust's parser. On
// Twitch, prediction outcomes and pinned messages take Rust's segments
// (parse_text_segments) and draw them through emoteWordMap.

import { test, vi } from 'vitest';
import assert from 'node:assert/strict';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
  convertFileSrc: (path: string) => `asset://localhost/${encodeURIComponent(path)}`,
}));
vi.mock('./assetCacheQueue', () => ({
  onAssetsCached: vi.fn(),
  requestAssetCaching: vi.fn(),
  setAssetCacheBurst: vi.fn(),
  forgetAssetRequests: vi.fn(),
}));

const { getEmoteLookup } = await import('./emoteService');
const { buildEmoteNameMap, emoteWords, emoteWordMap, EmoteText } = await import('../utils/emoteText');
const { createElement } = await import('react');
const { renderToStaticMarkup } = await import('react-dom/server');
type EmoteSet = import('./emoteService').EmoteSet;

const set: EmoteSet = {
  twitch: [{ id: '1035663', name: 'xqcL', url: 'https://static-cdn.jtvnw.net/emoticons/v2/1035663/default/dark/2.0', provider: 'twitch' }],
  '7tv': [
    { id: '7tv-xqcl', name: 'xqcL', url: 'https://cdn.7tv.app/emote/7tv-xqcl/1x.avif', provider: '7tv' },
    { id: '7tv-clash', name: 'Clash', url: 'https://cdn.7tv.app/emote/7tv-clash/1x.avif', provider: '7tv' },
    { id: '7tv-g-ez', name: 'EZ', url: 'https://cdn.7tv.app/emote/7tv-g-ez/1x.avif', provider: '7tv', emote_type: 'global' },
  ],
  bttv: [
    { id: 'bttv-clash', name: 'Clash', url: 'https://cdn.betterttv.net/emote/bttv-clash/1x', provider: 'bttv' },
    { id: 'bttv-ez', name: 'EZ', url: 'https://cdn.betterttv.net/emote/bttv-ez/1x', provider: 'bttv' },
    { id: 'bttv-dup', name: 'Dup', url: 'https://cdn.betterttv.net/emote/bttv-dup/1x', provider: 'bttv' },
    { id: 'bttv-g-dup', name: 'GDup', url: 'https://cdn.betterttv.net/emote/bttv-g-dup/1x', provider: 'bttv', emote_type: 'global' },
  ],
  ffz: [
    { id: 'ffz-xqcl', name: 'xqcL', url: 'https://cdn.frankerfacez.com/emote/1/1', provider: 'ffz' },
    { id: 'ffz-dup', name: 'Dup', url: 'https://cdn.frankerfacez.com/emote/2/1', provider: 'ffz' },
    { id: 'ffz-g-dup', name: 'GDup', url: 'https://cdn.frankerfacez.com/emote/3/1', provider: 'ffz', emote_type: 'global' },
  ],
  kick: [],
  youtube: [],
};

/** What Rust's chat parser draws for each shared name (EmoteLookup::build). */
function expectChatOrder(lookup: (name: string) => string | undefined) {
  assert.equal(lookup('xqcL'), '1035663');
  // Third-party sets: 7TV > FFZ > BTTV, the 7TV extension's order.
  assert.equal(lookup('Clash'), '7tv-clash');
  assert.equal(lookup('Dup'), 'ffz-dup');
  assert.equal(lookup('GDup'), 'ffz-g-dup');
  // A channel's own emote beats any global of the same name.
  assert.equal(lookup('EZ'), 'bttv-ez');
}

test('the chat row lookup resolves a shared name the way chat draws it', () => {
  const { byName } = getEmoteLookup(set);
  expectChatOrder((name) => byName.get(name)?.id);
});

test('the plain-text name map resolves a shared name the way chat draws it', () => {
  const map = buildEmoteNameMap(set);
  expectChatOrder((name) => map.get(name)?.id);
});

test('a pinned message draws the emote Rust chose for each word', () => {
  // ChatWidget draws the open pin and the collapsed pin bar through EmoteText
  // over Rust's segments for the text (useTextEmotes), word by word. Rust's
  // pick wins; the loaded set supplies the image it draws.
  const map = emoteWordMap(
    [
      { type: 'emote', content: 'xqcL', emote_id: '1035663', emote_url: 'https://static-cdn.jtvnw.net/emoticons/v2/1035663/default/dark/3.0' },
      { type: 'text', content: ' hello ' },
      { type: 'emote', content: 'Clash', emote_id: 'bttv-clash', emote_url: 'https://cdn.betterttv.net/emote/bttv-clash/3x' },
    ],
    set,
  );
  const words = emoteWords('xqcL hello Clash', map);
  assert.equal(words[0].emote?.id, '1035663');
  assert.equal(words[0].emote?.url, set.twitch[0].url);
  assert.equal(words[2].text, 'hello');
  assert.equal(words[2].emote, undefined);
  assert.equal(words[4].emote?.id, 'bttv-clash');
  assert.equal(words[4].emote?.provider, 'bttv');
  const html = renderToStaticMarkup(createElement(EmoteText, { text: 'xqcL', emoteMap: map }));
  assert.ok(html.includes(set.twitch[0].url), html);
  assert.ok(!html.includes('7tv-xqcl'), html);
});

test('an emote the loaded set lacks draws as Rust describes it', () => {
  const map = emoteWordMap(
    [{ type: 'emote', content: 'newOne', emote_id: 'n1', emote_url: 'https://cdn.7tv.app/emote/n1/2x.avif' }],
    set,
  );
  assert.equal(map.get('newOne')?.url, 'https://cdn.7tv.app/emote/n1/2x.avif');
  assert.equal(map.get('newOne')?.provider, '7tv');
});
