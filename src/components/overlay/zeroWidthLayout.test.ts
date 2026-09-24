// Run with: npm test
//
// Zero-width emotes (7TV's overlay flag) draw on top of the emote before them.
// The overlay used to lay them side by side, which turns a hat or a rain
// overlay into a stray second picture.

import { test } from 'vitest';
import assert from 'node:assert/strict';

import { zeroWidthLayout } from './overlayLayout';

type Seg = Parameters<typeof zeroWidthLayout>[0][number];

const emote = (content: string, zw = false, extra: Record<string, unknown> = {}): Seg =>
  ({ type: 'emote', content, emote_url: `https://cdn.7tv.app/emote/${content}/2x.webp`, is_zero_width: zw || undefined, ...extra }) as Seg;
const text = (content: string): Seg => ({ type: 'text', content }) as Seg;
const emoji = (content: string): Seg => ({ type: 'emoji', content, emoji_url: '' }) as Seg;

const entries = (m: Map<number, number[]>) => [...m.entries()];

test('an overlay across one space stacks on the emote before it', () => {
  const { attached, skip } = zeroWidthLayout([emote('KEKW'), text(' '), emote('RainTime', true)]);
  assert.deepEqual(entries(attached), [[0, [2]]]);
  assert.deepEqual([...skip].sort(), [1, 2], 'the overlay and the bridged space render inside the stack');
});

test('several overlays in a row share one base', () => {
  const { attached } = zeroWidthLayout([
    emote('peepoHappy'),
    text(' '),
    emote('SantaHat', true),
    text(' '),
    emote('RainTime', true),
  ]);
  assert.deepEqual(entries(attached), [[0, [2, 4]]]);
});

test('an emoji can carry an overlay too', () => {
  const { attached } = zeroWidthLayout([emoji('🍕'), emote('SantaHat', true)]);
  assert.deepEqual(entries(attached), [[0, [1]]]);
});

test('words break the stack, so a lone overlay renders as a normal emote', () => {
  const segs = [emote('KEKW'), text(' look '), emote('RainTime', true)];
  const { attached, skip } = zeroWidthLayout(segs);
  assert.equal(attached.size, 0);
  assert.equal(skip.size, 0);
  assert.equal(zeroWidthLayout([emote('RainTime', true)]).attached.size, 0, 'nothing to sit on at the start');
});

test('only one whitespace run is bridged', () => {
  const { attached } = zeroWidthLayout([emote('KEKW'), text(' '), text(' '), emote('RainTime', true)]);
  assert.equal(attached.size, 0);
});

test('an overlay that will render as its word stays in the text flow', () => {
  // A hidden personal emote shows the typed word; stacked, it would print over the emote.
  const personal = emote('myHat', true, { is_personal: true });
  const { attached, skip } = zeroWidthLayout([emote('KEKW'), text(' '), personal], (s) => s !== personal);
  assert.equal(attached.size, 0);
  assert.equal(skip.size, 0);
});
