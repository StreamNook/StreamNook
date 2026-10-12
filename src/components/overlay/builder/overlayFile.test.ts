// Run with: npm test
//
// Overlay files (the web builder's half; the desktop half is Rust,
// commands/overlay_file.rs). A file carries the look only, never the link,
// channels, or blocked users, and opening one always makes a new draft.

import { test } from 'vitest';
import assert from 'node:assert/strict';

import {
  MAX_OVERLAY_FILE_BYTES,
  OVERLAY_FILE_FORMAT,
  OVERLAY_FILE_VERSION,
  buildOverlayFile,
  overlayFileName,
  parseOverlayFile,
  profileFromImport,
} from './profileSync';
import { DEFAULT_OVERLAY_STYLE, type OverlayStyle } from '../overlayConfig';

const senderStyle = (): OverlayStyle => ({
  ...DEFAULT_OVERLAY_STYLE,
  fontFamily: "'Bebas Neue', sans-serif",
  fontSize: 22,
  bubble: true,
  entrance: 'pop',
  hidePhrases: ['spoiler'],
  blockedUsers: { 'twitch:sender': ['someone'] },
  ...({ profileName: "Sender's overlay" } as object),
});

test('round trip keeps the look and leaves account data behind', () => {
  const text = buildOverlayFile('My overlay', senderStyle());
  const doc = JSON.parse(text);
  assert.equal(doc.format, OVERLAY_FILE_FORMAT);
  assert.equal(doc.version, OVERLAY_FILE_VERSION);
  assert.equal(doc.style.blockedUsers, undefined);
  assert.equal(doc.style.profileName, undefined);
  assert.equal(doc.id, undefined);
  assert.equal(doc.channels, undefined);

  const file = parseOverlayFile(text);
  assert.equal(file.name, 'My overlay');
  assert.deepEqual(file.dropped, []);
  assert.equal(file.style.fontFamily, "'Bebas Neue', sans-serif");
  assert.equal(file.style.fontSize, 22);
  assert.equal(file.style.entrance, 'pop');
  assert.deepEqual(file.style.hidePhrases, ['spoiler']);
});

test('an import is a new draft that keeps the receiver sources and blocked users', () => {
  const file = parseOverlayFile(buildOverlayFile('Shared', senderStyle()));
  const mine = { ...DEFAULT_OVERLAY_STYLE, blockedUsers: { 'kick:me': ['troll'] } };
  const p = profileFromImport(file, 'Shared', { sources: [{ provider: 'kick', channel: 'me' }], style: mine });
  assert.equal(p.id, null);
  assert.equal(p.version, null);
  assert.deepEqual(p.sources, [{ provider: 'kick', channel: 'me' }]);
  assert.deepEqual(p.style.blockedUsers, { 'kick:me': ['troll'] });
  assert.equal(p.style.fontSize, 22);
});

test('wrong types, unknown keys and CSS break-outs are dropped', () => {
  const file = parseOverlayFile(
    JSON.stringify({
      format: OVERLAY_FILE_FORMAT,
      version: 1,
      name: '  \u0007Bad name ',
      style: {
        fontSize: 'huge',
        fontFamily: 'x; background: url(https://evil)',
        bodyTextColor: 'red; position: fixed',
        linkColor: '',
        bubble: 'yes',
        hidePhrases: 'spoiler',
        mystery: true,
        blockedUsers: { 'twitch:x': ['y'] },
        width: 500,
      },
    }),
  );
  assert.equal(file.name, 'Bad name');
  assert.equal(file.style.width, 500);
  assert.equal(file.style.linkColor, '');
  assert.deepEqual(file.dropped, ['blockedUsers', 'bodyTextColor', 'bubble', 'fontFamily', 'fontSize', 'hidePhrases', 'mystery']);
});

test('refuses files that are not overlay files', () => {
  assert.throws(() => parseOverlayFile('not json'), /not a StreamNook overlay file/);
  assert.throws(() => parseOverlayFile('[]'), /not a StreamNook overlay file/);
  assert.throws(() => parseOverlayFile(JSON.stringify({ format: 'other', version: 1, style: {} })), /not a StreamNook/);
  assert.throws(() => parseOverlayFile(JSON.stringify({ format: OVERLAY_FILE_FORMAT, style: {} })), /no version/);
  assert.throws(() => parseOverlayFile(JSON.stringify({ format: OVERLAY_FILE_FORMAT, version: 1 })), /no settings/);
  assert.throws(
    () => parseOverlayFile(JSON.stringify({ format: OVERLAY_FILE_FORMAT, version: OVERLAY_FILE_VERSION + 1, style: {} })),
    /newer StreamNook/,
  );
  assert.throws(() => parseOverlayFile('x'.repeat(MAX_OVERLAY_FILE_BYTES + 1)), /too large/);
});

test('file names are safe and carry the extension', () => {
  assert.equal(overlayFileName('a/b:c*?'), 'a b c.snoverlay');
  assert.equal(overlayFileName('   '), 'Imported overlay.snoverlay');
});
