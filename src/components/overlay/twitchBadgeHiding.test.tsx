// @vitest-environment node
// Run with: npm test
//
// Per-category Twitch badge hiding, end to end through the renderer, plus the
// parts of the backend's badge table that twitchBadgeCategories.test.ts does not
// compare: the sets no category covers, and the catch-all id.

import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { renderToStaticMarkup } from 'react-dom/server';

import { OverlayChat } from './OverlayChat';
import { SAMPLE_MESSAGES, type OverlayMessage } from './sampleMessages';
import {
  DEFAULT_OVERLAY_STYLE,
  TWITCH_BADGE_CATEGORIES,
  clampOverlayStyle,
  twitchBadgeCategory,
  type OverlayStyle,
} from './overlayConfig';

const root = resolve(__dirname, '..', '..', '..');
const rustSrc = readFileSync(resolve(root, 'src-tauri/src/models/chat_layout.rs'), 'utf8');
const tsSrc = readFileSync(resolve(root, 'src/components/overlay/overlayConfig.ts'), 'utf8');

const quoted = (block: string) => [...block.matchAll(/["']([^"']+)["']/g)].map((m) => m[1]).sort();

const BADGE = (id: string) => `https://static-cdn.jtvnw.net/badges/v1/${id}/3`;
const badges: OverlayMessage['badges'] = [
  { name: 'broadcaster', version: '1', image_url_4x: BADGE('b'), title: 'Broadcaster' },
  { name: 'moderator', version: '1', image_url_4x: BADGE('m'), title: 'Moderator' },
  { name: 'predictions', version: 'blue-1', image_url_4x: BADGE('p'), title: 'Predicted Blue (1)' },
  { name: 'subscriber', version: '12', image_url_4x: BADGE('s'), title: '1-Year Subscriber' },
  { name: 'premium', version: '1', image_url_4x: BADGE('x'), title: 'Prime Gaming' },
];
const template = SAMPLE_MESSAGES.find((m) => (m.provider ?? 'twitch') === 'twitch' && (m.badges ?? []).length > 0)!;
const row = (provider: OverlayMessage['provider']): OverlayMessage => ({ ...template, id: `badge-${provider}`, provider, badges });

const render = (provider: OverlayMessage['provider'], style: Partial<OverlayStyle>) =>
  renderToStaticMarkup(<OverlayChat messages={[row(provider)]} style={{ ...DEFAULT_OVERLAY_STYLE, ...style }} />);
const shown = (html: string) => badges.filter((b) => html.includes(`alt="${b.title}"`)).map((b) => b.name);

describe('Twitch badge categories in the renderer', () => {
  it('shows every badge on a config saved before the field existed', () => {
    const legacy: Partial<OverlayStyle> = { ...DEFAULT_OVERLAY_STYLE };
    delete legacy.hiddenTwitchBadgeCategories;
    const html = renderToStaticMarkup(<OverlayChat messages={[row('twitch')]} style={legacy as OverlayStyle} />);
    expect(shown(html)).toEqual(badges.map((b) => b.name));
  });

  it('hides only the chosen categories on a Twitch row', () => {
    expect(shown(render('twitch', { hiddenTwitchBadgeCategories: ['predictions'] }))).toEqual(
      ['broadcaster', 'moderator', 'subscriber', 'premium'],
    );
    expect(shown(render('twitch', { hiddenTwitchBadgeCategories: ['roles', 'other'] }))).toEqual(
      ['broadcaster', 'predictions', 'subscriber'],
    );
  });

  it('keeps the broadcaster badge with every category hidden', () => {
    const all = TWITCH_BADGE_CATEGORIES.map((c) => c.id);
    expect(shown(render('twitch', { hiddenTwitchBadgeCategories: all }))).toEqual(['broadcaster']);
  });

  it('leaves other platforms alone even when they reuse a Twitch set name', () => {
    const all = TWITCH_BADGE_CATEGORIES.map((c) => c.id);
    expect(shown(render('kick', { hiddenTwitchBadgeCategories: all }))).toEqual(badges.map((b) => b.name));
  });

  it('drops unknown category ids instead of hiding anything with them', () => {
    const style = clampOverlayStyle({ ...DEFAULT_OVERLAY_STYLE, hiddenTwitchBadgeCategories: ['broadcaster', 'PREDICTIONS'] });
    expect(style.hiddenTwitchBadgeCategories).toEqual([]);
  });
});

describe('Twitch badge table, the parts the category drift test skips', () => {
  it('lists the same never-categorised sets as the backend', () => {
    const rustAt = rustSrc.indexOf('TWITCH_BADGE_SETS_NEVER_CATEGORISED: &[&str] = &[');
    expect(rustAt).toBeGreaterThan(-1);
    const rust = quoted(rustSrc.slice(rustAt, rustSrc.indexOf('];', rustAt)).split('= &[')[1]);
    const tsAt = tsSrc.indexOf('TWITCH_BADGE_SETS_NEVER_CATEGORISED = new Set([');
    expect(tsAt).toBeGreaterThan(-1);
    const ts = quoted(tsSrc.slice(tsAt, tsSrc.indexOf('])', tsAt)).split('new Set([')[1]);
    expect(rust.length).toBeGreaterThan(0);
    expect(ts).toEqual(rust);
    for (const set of rust) expect(twitchBadgeCategory(set)).toBeNull();
  });

  it('uses the same catch-all id as the backend', () => {
    const m = /TWITCH_BADGE_CATEGORY_OTHER: &str = "([^"]+)"/.exec(rustSrc);
    expect(m).not.toBeNull();
    expect(TWITCH_BADGE_CATEGORIES.some((c) => c.id === m![1] && c.sets.length === 0)).toBe(true);
    expect(twitchBadgeCategory('some-future-badge')).toBe(m![1]);
  });
});
