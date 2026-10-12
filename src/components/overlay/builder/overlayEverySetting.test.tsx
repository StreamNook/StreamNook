// @vitest-environment node
// Run with: npm test
//
// Every overlay setting survives an overlay file unchanged and draws the same
// after import. The fixture changes every setting from its default and covers
// every option of every choice and both ends of every number range; the Rust
// half (commands/overlay_file.rs) round-trips the same fixture through the
// desktop file path.

import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

import fixture from './fixtures/overlayEverySetting.json';
import { buildOverlayFile, parseOverlayFile, profileFromImport } from './profileSync';
import { OverlayChat } from '../OverlayChat';
import { SAMPLE_MESSAGES } from '../sampleMessages';
import {
  BUBBLE_SHAPES,
  CHEER_DISPLAYS,
  DEFAULT_OVERLAY_STYLE,
  EMOJI_STYLES,
  GIANT_EMOTE_ALIGNS,
  LINK_STYLES,
  OVERLAY_ANIMATIONS,
  OVERLAY_ENTRANCES,
  OVERLAY_LIMITS,
  OVERLAY_TEXT_ALIGNS,
  OVERLAY_TEXT_WEIGHTS,
  REPLY_STYLES,
  clampOverlayStyle,
  type OverlayStyle,
} from '../overlayConfig';

type Style = Record<string, unknown>;

/** Keys a file never carries, or that the builder always keeps empty. */
const NOT_IN_FILES = new Set(['blockedUsers', 'profileName']);
/** Legacy: folded into hiddenProviderEvents on load, so always [] in a style. */
const ALWAYS_EMPTY = new Set(['hiddenEvents']);

const base = fixture.base as Style;
const styles: { label: string; style: Style }[] = [
  { label: 'every setting changed', style: base },
  ...fixture.variants.map((v, i) => ({ label: `variant ${i + 1}`, style: { ...base, ...(v as Style) } })),
];

const asStyle = (s: Style) => ({ ...DEFAULT_OVERLAY_STYLE, ...s }) as OverlayStyle;
const render = (s: OverlayStyle) => renderToStaticMarkup(<OverlayChat messages={SAMPLE_MESSAGES} style={s} />);

describe('overlay file: every setting', () => {
  it('the fixture changes every setting the builder has', () => {
    const defaults = DEFAULT_OVERLAY_STYLE as unknown as Style;
    for (const key of Object.keys(defaults)) {
      if (NOT_IN_FILES.has(key)) continue;
      expect(base, `fixture is missing ${key}`).toHaveProperty(key);
      if (ALWAYS_EMPTY.has(key)) continue;
      expect(base[key], `fixture leaves ${key} at its default`).not.toEqual(defaults[key]);
    }
    for (const key of Object.keys(base)) {
      expect(defaults, `fixture has ${key}, which the overlay no longer has`).toHaveProperty(key);
    }
  });

  it('covers every option of every choice and both ends of every range', () => {
    const seen = (key: string) => new Set(styles.map((s) => String(s.style[key] ?? (DEFAULT_OVERLAY_STYLE as unknown as Style)[key])));
    const choices: [string, string[]][] = [
      ['sourceTag', ['none', 'dot', 'label', 'icon']],
      ['background', ['transparent', 'solid']],
      ['eventStyle', ['plain', 'streamnook', 'outline']],
      ['firstTimeStyle', ['off', 'twitch', 'streamnook']],
      ['direction', ['newBottom', 'newTop']],
      ['entrance', OVERLAY_ENTRANCES.map((o) => o.value)],
      ['textAlign', OVERLAY_TEXT_ALIGNS.map((o) => o.value)],
      ['giantEmoteAlign', GIANT_EMOTE_ALIGNS.map((o) => o.value)],
      ['eventAnimation', OVERLAY_ANIMATIONS.map((o) => o.value)],
      ['firstTimeAnimation', OVERLAY_ANIMATIONS.map((o) => o.value)],
      ['bubbleShape', BUBBLE_SHAPES.map((o) => o.value)],
      ['replyStyle', REPLY_STYLES.map((o) => o.value)],
      ['linkStyle', LINK_STYLES.map((o) => o.value)],
      ['cheerDisplay', CHEER_DISPLAYS.map((o) => o.value)],
      ['emojiStyle', EMOJI_STYLES.map((o) => o.value)],
      ['fontWeight', OVERLAY_TEXT_WEIGHTS.map((o) => o.value)],
    ];
    for (const [key, options] of choices) {
      const covered = seen(key);
      // The default counts: the default style is what an unchanged setting exports.
      covered.add(String((DEFAULT_OVERLAY_STYLE as unknown as Style)[key]));
      for (const option of options) expect(covered.has(option), `${key}: "${option}" never round-trips`).toBe(true);
    }
    for (const [key, { min, max }] of Object.entries(OVERLAY_LIMITS)) {
      const values = styles.map((s) => s.style[key]);
      expect(values, `${key} min`).toContain(min);
      expect(values, `${key} max`).toContain(max);
    }
  });

  for (const { label, style } of styles) {
    it(`${label}: the builder keeps every value as is`, () => {
      const clamped = clampOverlayStyle(asStyle(style)) as unknown as Style;
      for (const [key, value] of Object.entries(style)) expect(clamped[key], key).toEqual(value);
    });

    it(`${label}: a file round-trip brings back every value`, () => {
      const file = parseOverlayFile(buildOverlayFile('Every setting', asStyle(style)));
      expect(file.dropped).toEqual([]);
      for (const [key, value] of Object.entries(style)) expect((file.style as Style)[key], key).toEqual(value);
    });

    it(`${label}: the imported overlay draws exactly like the original`, () => {
      const receiver = { sources: [], style: { ...DEFAULT_OVERLAY_STYLE, blockedUsers: { 'twitch:me': ['troll'] } } };
      const file = parseOverlayFile(buildOverlayFile('Every setting', asStyle(style)));
      const imported = profileFromImport(file, 'Every setting', receiver).style;
      for (const [key, value] of Object.entries(style)) expect((imported as unknown as Style)[key], key).toEqual(value);
      expect(imported.blockedUsers).toEqual({ 'twitch:me': ['troll'] });
      const original = clampOverlayStyle({ ...asStyle(style), blockedUsers: imported.blockedUsers });
      expect(render(imported)).toBe(render(original));
    });
  }
});
