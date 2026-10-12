// Run with: npm test
//
// Text that reaches the page without Twitch's emote positions (a local echo
// before the server's copy, a reply preview, prediction outcomes, pinned
// messages) resolves names through these lookups. A Twitch emote must win
// over a third-party emote with the same name, as it does in Rust's parser.

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
const { buildEmoteNameMap, emoteWords, EmoteText } = await import('../utils/emoteText');
const { createElement } = await import('react');
const { renderToStaticMarkup } = await import('react-dom/server');
type EmoteSet = import('./emoteService').EmoteSet;

const set: EmoteSet = {
  twitch: [{ id: '1035663', name: 'xqcL', url: 'https://static-cdn.jtvnw.net/emoticons/v2/1035663/default/dark/2.0', provider: 'twitch' }],
  '7tv': [
    { id: '7tv-xqcl', name: 'xqcL', url: 'https://cdn.7tv.app/emote/7tv-xqcl/1x.avif', provider: '7tv' },
    { id: '7tv-clash', name: 'Clash', url: 'https://cdn.7tv.app/emote/7tv-clash/1x.avif', provider: '7tv' },
  ],
  bttv: [{ id: 'bttv-clash', name: 'Clash', url: 'https://cdn.betterttv.net/emote/bttv-clash/1x', provider: 'bttv' }],
  ffz: [{ id: 'ffz-xqcl', name: 'xqcL', url: 'https://cdn.frankerfacez.com/emote/1/1', provider: 'ffz' }],
  kick: [],
  youtube: [],
};

test('the chat row lookup resolves a shared name to the Twitch emote', () => {
  const { byName } = getEmoteLookup(set);
  assert.equal(byName.get('xqcL')?.id, '1035663');
  // Third-party sets keep 7TV > BTTV > FFZ among themselves.
  assert.equal(byName.get('Clash')?.id, '7tv-clash');
});

test('the plain-text name map resolves a shared name to the Twitch emote', () => {
  const map = buildEmoteNameMap(set);
  assert.equal(map.get('xqcL')?.id, '1035663');
  assert.equal(map.get('Clash')?.id, '7tv-clash');
});

test('a pinned message draws the Twitch emote for a shared name', () => {
  // ChatWidget draws the open pin and the collapsed pin bar through EmoteText
  // over buildEmoteNameMap (renderPinTextWithEmotes), word by word.
  const map = buildEmoteNameMap(set);
  const words = emoteWords('xqcL hello Clash', map);
  assert.equal(words[0].emote?.id, '1035663');
  assert.equal(words[2].text, 'hello');
  assert.equal(words[2].emote, undefined);
  assert.equal(words[4].emote?.id, '7tv-clash');
  const html = renderToStaticMarkup(createElement(EmoteText, { text: 'xqcL', emoteMap: map }));
  assert.ok(html.includes(set.twitch[0].url), html);
  assert.ok(!html.includes('7tv-xqcl'), html);
});
