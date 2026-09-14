// Run with: npm test
//
// Every input below is copied from a real badge's earn text. The possessive
// case matters most: the previous pattern only accepted a straight apostrophe,
// while 126 badges in the catalogue use the curly one, so it never matched.

import { test } from 'vitest';
import assert from 'node:assert/strict';

import { extractChannelLogins, splitChannelMentions } from './badgeChannels.ts';

test('finds a bare slash handle', () => {
  assert.deepEqual(
    extractChannelLogins('Subscribe or gift a sub to /studbudz during WNBA All-Star Weekend'),
    ['studbudz']
  );
});

test('finds a channel named in prose without a slash', () => {
  const text =
    'To earn this badge, subscribe to or gift a subscription to the participating channel StudBudz.';
  assert.deepEqual(extractChannelLogins(text), ['studbudz']);
});

test('finds possessives with either apostrophe', () => {
  assert.deepEqual(extractChannelLogins("watch Ibai's channel"), ['ibai']);
  assert.deepEqual(extractChannelLogins('watch Ibai’s channel'), ['ibai']);
  assert.deepEqual(
    extractChannelLogins('earned by watching 30 minutes of JasonTheWeen’s stream'),
    ['jasontheween']
  );
});

test('finds every channel in a twitch.tv list', () => {
  const text = 'on either of the following channels: twitch.tv/fps_shaka twitch.tv/legendus_shaka';
  const got = extractChannelLogins(text);
  assert.ok(got.includes('fps_shaka'));
  assert.ok(got.includes('legendus_shaka'));
});

test('splits a multi-channel phrase', () => {
  assert.deepEqual(
    extractChannelLogins('watch the channels Alpha_One and BetaTwo to qualify').sort(),
    ['alpha_one', 'betatwo']
  );
});

test('ignores site routes that are not channels', () => {
  assert.deepEqual(extractChannelLogins('see https://www.twitch.tv/directory/event/football-fest'), []);
  assert.deepEqual(extractChannelLogins('Twitch turbo page: https://www.twitch.tv/turbo'), []);
});

test('ignores generic words after "channel"', () => {
  assert.deepEqual(extractChannelLogins('any participating channel during the event'), []);
  assert.deepEqual(extractChannelLogins('subscribe to an eligible channel'), []);
  assert.deepEqual(extractChannelLogins('the streamer’s channel must be live'), []);
});

test('does not invent a channel from a date or a bare number', () => {
  assert.deepEqual(extractChannelLogins('Event duration: 2026-07-08 - 2026-08-24'), []);
});

test('dedupes a channel named more than one way', () => {
  const text = 'Subscribe to /studbudz. The participating channel StudBudz must be live.';
  assert.deepEqual(extractChannelLogins(text), ['studbudz']);
});

// The panel chips these mentions inline, so a slash that is not a mention has
// to stay prose. Both inputs are from the Pokémon chat-badge campaign copy,
// which put chips on "LeafGreen", "Violet", "article" and "pokemon".
test('a slash inside a word is not a channel', () => {
  const text = [
    'Full list of eligible categories:',
    'Pokémon FireRed/LeafGreen, Pokémon Pokopia, Pokémon UNITE, Pokémon Champions, Pokémon GO,',
    'Pokémon Legends: Z-A, Pokémon Scarlet/Violet, Pokémon Trading Card Game Pocket, Just Chatting,',
    'DJs, Art, Special Events, Sports, Music, Talk Shows & Podcasts, Animals, Aquariums, and Zoos,',
    'Co-working & Studying, and Makers & Crafting',
  ].join(' ');
  assert.deepEqual(extractChannelLogins(text), []);
  assert.deepEqual(splitChannelMentions(text).filter(p => p.login), []);
});

test('a URL path is not a list of channels', () => {
  const text = 'Official description: https://help.twitch.tv/s/article/pokemon-chat-badges';
  assert.deepEqual(extractChannelLogins(text), []);
  assert.deepEqual(splitChannelMentions(text).filter(p => p.login), []);
});

test('splitting keeps the prose intact around a mention', () => {
  const text = 'Subscribe to /studbudz during the event.';
  const parts = splitChannelMentions(text);
  assert.equal(parts.map(p => p.text).join(''), text);
  assert.deepEqual(parts.filter(p => p.login).map(p => p.login), ['studbudz']);
});

test('a twitch.tv link chips as one whole mention', () => {
  const parts = splitChannelMentions('watch https://www.twitch.tv/fps_shaka live');
  assert.deepEqual(
    parts.map(p => (p.login ? `<${p.login}>` : p.text)),
    ['watch ', '<fps_shaka>', ' live']
  );
});

// From the Pichu badge. The list below "channels:" names the three sub badges,
// not channels, and the old rule read it as one: it backtracked into the name
// until its lookahead passed and offered the login "Bulbasau", which is a real
// but unrelated channel, so the panel carded a stranger.
test('a list below "channels:" is not a list of channels', () => {
  const text = [
    'The campaign also features three additional Pokémon Chat Badges that require',
    'a subscription to participating channels:',
    '',
    'Bulbasaur',
    'Charmander',
    'Squirtle',
  ].join('\n');
  assert.deepEqual(extractChannelLogins(text), []);
});

test('a name is never truncated to make a match fit', () => {
  // "Riot Games" is a display name, not a login, so the whole match is refused
  // rather than shortened to "Riot".
  assert.deepEqual(extractChannelLogins('watch the channels Riot Games'), []);
});
