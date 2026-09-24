// Run with: npm test
//
// Two shapes of YouTube parsing exist on purpose. Collapsing them into one has a
// specific consequence: the tolerant version resolves a bare word as a handle,
// so using it where a typed name should stay a Twitch login makes every
// add-by-name silently become a YouTube lookup instead.

import { test } from 'vitest';
import assert from 'node:assert/strict';

import {
  isTwitchLogin,
  isYouTubeChannelId,
  isYouTubeLegacyPath,
  kickSlugFromInput,
  kickSlugHasTwoSpellings,
  linkPlatformOf,
  parseChannelInput,
  parseKickLink,
  parseTwitchLink,
  parseLinkInput,
  parseTikTokIdentifier,
  parseTikTokLink,
  parseYouTubeIdentifier,
  parseYouTubeLink,
} from './parseChannelInput.ts';

test('the link parser ignores a bare word, so a typed name stays Twitch', () => {
  assert.equal(parseYouTubeLink('xqc'), null);
  assert.equal(parseYouTubeLink('@mrbeast'), null);
  assert.equal(parseYouTubeLink('UCX6OQ3DkcsbYNE6H8uQQuVA'), null);
  // MultiChat's add box asks TikTok next, so its link parser must hold the same line.
  assert.equal(parseTikTokLink('xqc'), null);
  assert.equal(parseTikTokLink('@xqc'), null);
});

test('the identifier parser accepts what someone would actually type', () => {
  assert.equal(parseYouTubeIdentifier('@mrbeast'), '@mrbeast');
  assert.equal(parseYouTubeIdentifier('mrbeast'), '@mrbeast');
  assert.equal(parseYouTubeIdentifier(''), null);
});

test('a UC channel id is told apart from every other YouTube identifier', () => {
  assert.equal(isYouTubeChannelId('UCX6OQ3DkcsbYNE6H8uQQuVA'), true);
  for (const id of ['@MrBeast', 'jfKfPfyJRdk', 'c/LofiGirl', 'UCtooShort', 'ucx6oq3dkcsbyne6h8uqquva']) {
    assert.equal(isYouTubeChannelId(id), false, id);
  }
});

test('a UC id keeps its case, because YouTube ids are case-sensitive', () => {
  const id = 'UCX6OQ3DkcsbYNE6H8uQQuVA';
  assert.equal(parseYouTubeIdentifier(id), id);
  assert.equal(parseYouTubeIdentifier(`  ${id}  `), id, 'and survives stray spacing');
});

test('YouTube link shapes all resolve', () => {
  assert.equal(parseYouTubeLink('https://youtu.be/jfKfPfyJRdk'), 'jfKfPfyJRdk');
  assert.equal(parseYouTubeLink('https://www.youtube.com/watch?v=jfKfPfyJRdk'), 'jfKfPfyJRdk');
  assert.equal(parseYouTubeLink('https://www.youtube.com/live/jfKfPfyJRdk'), 'jfKfPfyJRdk');
  assert.equal(parseYouTubeLink('https://youtube.com/shorts/jfKfPfyJRdk'), 'jfKfPfyJRdk');
  // The popout chat link, the one people copy into OBS.
  assert.equal(
    parseYouTubeLink('https://www.youtube.com/live_chat?is_popout=1&v=jfKfPfyJRdk'),
    'jfKfPfyJRdk',
  );
  assert.equal(
    parseYouTubeLink('https://www.youtube.com/channel/UCX6OQ3DkcsbYNE6H8uQQuVA'),
    'UCX6OQ3DkcsbYNE6H8uQQuVA',
  );
  assert.equal(parseYouTubeLink('https://www.youtube.com/@MrBeast'), '@MrBeast');
  assert.equal(parseYouTubeLink('https://www.youtube.com/@MrBeast/live'), '@MrBeast');
  // Punctuation trailing a link pasted out of a sentence is not part of it.
  assert.equal(parseYouTubeLink('https://www.youtube.com/@MrBeast,'), '@MrBeast');
});

test('a legacy /c/ or /user/ link comes back as its path, never as a guessed handle', () => {
  // The old name is not a handle: /user/MrBeast6000 is MrBeast, while
  // @MrBeast6000 does not exist. So it is resolved, never rewritten.
  assert.equal(parseYouTubeLink('https://www.youtube.com/user/MrBeast6000'), 'user/MrBeast6000');
  assert.equal(parseYouTubeLink('https://www.youtube.com/c/LofiGirl/live'), 'c/LofiGirl');
  assert.equal(parseYouTubeLink('https://youtube.com/C/LofiGirl'), 'c/LofiGirl');
  assert.equal(parseYouTubeIdentifier('https://www.youtube.com/c/LofiGirl'), 'c/LofiGirl');
  assert.deepEqual(parseChannelInput('https://www.youtube.com/c/LofiGirl'), {
    provider: 'youtube',
    channel: 'c/LofiGirl',
  });
  assert.ok(isYouTubeLegacyPath('c/LofiGirl'));
  assert.ok(isYouTubeLegacyPath('user/MrBeast6000'));
  for (const id of ['@LofiGirl', 'UCX6OQ3DkcsbYNE6H8uQQuVA', 'jfKfPfyJRdk']) {
    assert.equal(isYouTubeLegacyPath(id), false, id);
  }
});

test('a handle can be in any script YouTube supports, typed or linked', () => {
  assert.equal(parseYouTubeIdentifier('ねこ'), '@ねこ');
  assert.equal(parseYouTubeIdentifier('@ねこ'), '@ねこ');
  assert.equal(parseYouTubeLink('https://www.youtube.com/@ねこ/live'), '@ねこ');
  // What a browser actually copies for that same link.
  assert.equal(parseYouTubeLink('https://www.youtube.com/@%E3%81%AD%E3%81%93'), '@ねこ');
  assert.equal(parseYouTubeIdentifier('l\xB7l'), '@l\xB7l', 'the Latin middle dot is allowed');
  assert.equal(parseYouTubeIdentifier('Lofi Girl'), null, 'a handle has no spaces');
});

test('a typed 11-character word is a handle, not a video id', () => {
  // A bare video id looks exactly like a handle. Reading one as a video would
  // make every 11-character handle unreachable, and a video has a link to paste.
  assert.equal(parseYouTubeIdentifier('3blue1brown'), '@3blue1brown');
  assert.equal(parseYouTubeIdentifier('https://youtu.be/jfKfPfyJRdk'), 'jfKfPfyJRdk');
});

test('TikTok has the same two shapes: a link, or a handle once TikTok is chosen', () => {
  const live = 'https://www.tiktok.com/@some.creator/live';
  assert.equal(parseTikTokLink(live), 'some.creator');
  assert.equal(parseTikTokIdentifier(live), 'some.creator');
  assert.equal(parseTikTokIdentifier('@some.creator'), 'some.creator');
  assert.equal(parseTikTokIdentifier('some.creator'), 'some.creator');
  assert.equal(parseTikTokIdentifier(''), null);
  assert.equal(parseTikTokIdentifier('not a handle'), null);
});

test('Kick and Twitch links fold to their canonical identifier', () => {
  assert.equal(parseKickLink('https://kick.com/xQc'), 'xqc');
  assert.equal(parseKickLink('kick:xQc'), 'xqc');
  assert.equal(parseKickLink('xqc'), null, 'a bare word is not a Kick link');
  // Some slugs carry a hyphen where the username has an underscore.
  assert.equal(parseKickLink('https://kick.com/some-name'), 'some-name');
  assert.equal(parseTwitchLink('https://www.twitch.tv/xQc'), 'xqc');
  assert.equal(parseTwitchLink('twitch:xQc'), 'xqc');
});

test('a Kick name is cleaned to what a slug holds, and flagged when it has two spellings', () => {
  assert.equal(kickSlugFromInput('ice poseidon'), 'iceposeidon');
  assert.equal(kickSlugFromInput('  @xQc '), 'xqc');
  assert.equal(kickSlugFromInput('https://kick.com/some-name'), 'some-name');
  assert.equal(kickSlugFromInput('Some_Name'), 'some_name');
  // Kick answers only the exact slug, and a username's underscore may be a
  // hyphen in its slug, so only these need asking which.
  assert.equal(kickSlugHasTwoSpellings('some_name'), true);
  assert.equal(kickSlugHasTwoSpellings('some-name'), true);
  assert.equal(kickSlugHasTwoSpellings('xqc'), false);
});

test('a pasted link names its own platform', () => {
  assert.deepEqual(parseChannelInput('https://kick.com/xqc'), { provider: 'kick', channel: 'xqc' });
  assert.deepEqual(parseChannelInput('https://twitch.tv/xqc'), {
    provider: 'twitch',
    channel: 'xqc',
  });
  assert.deepEqual(parseChannelInput('https://www.youtube.com/@MrBeast'), {
    provider: 'youtube',
    channel: '@MrBeast',
  });
  assert.equal(parseChannelInput('xqc'), null, 'a bare word names no platform');
});

test('a link overrides a platform picker, a bare word does not', () => {
  // The overlay's add box has a platform dropdown. A kick.com link pasted with
  // Twitch selected must land on Kick, not become a Twitch channel named after
  // the URL.
  assert.equal(linkPlatformOf('https://kick.com/xqc'), 'kick');
  assert.equal(linkPlatformOf('  https://www.twitch.tv/xqc  '), 'twitch');
  assert.equal(linkPlatformOf('https://www.youtube.com/@MrBeast'), 'youtube');
  assert.equal(linkPlatformOf('https://www.youtube.com/c/LofiGirl'), 'youtube');
  assert.equal(linkPlatformOf('https://www.tiktok.com/@someone/live'), 'tiktok');
  assert.equal(linkPlatformOf('xqc'), null, 'a bare word keeps the picked platform');
  assert.equal(linkPlatformOf('@MrBeast'), null, 'a handle keeps the picked platform');
});

test('a Twitch login is letters, digits and underscores only', () => {
  assert.equal(isTwitchLogin('xqc'), true);
  assert.equal(isTwitchLogin('some_name_123'), true);
  assert.equal(isTwitchLogin('https://kick.com/xqc'), false);
  assert.equal(isTwitchLogin('two words'), false);
  assert.equal(isTwitchLogin('a'.repeat(26)), false);
  assert.equal(isTwitchLogin(''), false);
});

test('the add box reads a bare name as Kick and a marked one as YouTube', () => {
  // A bare word has to mean SOMETHING, and Kick is the platform people share by
  // name. YouTube channels are addressed by an id or an @handle, both of which
  // are unambiguous on their own, so neither is stolen by this default.
  assert.deepEqual(parseLinkInput('xqc'), { provider: 'kick', channel: 'xqc' });
  assert.deepEqual(parseLinkInput('@MrBeast'), { provider: 'youtube', channel: '@MrBeast' });
  assert.deepEqual(parseLinkInput('UCX6OQ3DkcsbYNE6H8uQQuVA'), {
    provider: 'youtube',
    channel: 'UCX6OQ3DkcsbYNE6H8uQQuVA',
  });
});

test('a pasted link in the add box always wins over the bare-name default', () => {
  assert.deepEqual(parseLinkInput('https://twitch.tv/xqc'), { provider: 'twitch', channel: 'xqc' });
  assert.deepEqual(parseLinkInput('https://www.youtube.com/@MrBeast'), {
    provider: 'youtube',
    channel: '@MrBeast',
  });
  assert.equal(parseLinkInput('   '), null);
});
