// Run with: npm test
//
// Readable name colors steer toward whatever the names sit on. On an OBS
// overlay that is the bubble, the solid panel, or the stream under the text
// shadow, in that order.

import { test } from 'vitest';
import assert from 'node:assert/strict';

import { overlayBackdropIsDark } from './overlayLayout';
import { DEFAULT_OVERLAY_STYLE, type OverlayStyle } from './overlayConfig';
import { adjustNameColor, nameColorReadsLight } from '../../utils/nameColor';

const style = (patch: Partial<OverlayStyle>): OverlayStyle => ({ ...DEFAULT_OVERLAY_STYLE, ...patch });

test('a transparent overlay under the default black shadow is dark', () => {
  assert.equal(overlayBackdropIsDark(style({})), true);
});

test('a light shadow means names sit on light', () => {
  assert.equal(overlayBackdropIsDark(style({ textShadowColor: '#ffffff' })), false);
  assert.equal(overlayBackdropIsDark(style({ textShadow: false, textShadowColor: '#ffffff' })), true, 'an unused shadow color is ignored');
});

test('a solid panel decides, and a bubble beats the panel', () => {
  assert.equal(overlayBackdropIsDark(style({ background: 'solid', backgroundColor: '#f4f4f5' })), false);
  assert.equal(
    overlayBackdropIsDark(style({ background: 'solid', backgroundColor: '#f4f4f5', bubble: true, bubbleColor: '#0e0e10' })),
    true,
  );
});

test('a navy name comes out readable on the default overlay', () => {
  const out = adjustNameColor('#000080', 'hsl_loop', overlayBackdropIsDark(style({})));
  assert.notEqual(out, '#000080');
  assert.equal(nameColorReadsLight(out), true);
});
