// @vitest-environment node
// Run with: npm test
//
// Badge providers added after overlays were in use (OPT_IN_BADGE_PROVIDERS)
// start off: an overlay draws them only once its streamer switches them on,
// while the older providers still show unless hidden.

import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

import { OverlayChat } from './OverlayChat';
import { SAMPLE_MESSAGES, type OverlayMessage } from './sampleMessages';
import {
  DEFAULT_OVERLAY_STYLE,
  OPT_IN_BADGE_PROVIDERS,
  THIRD_PARTY_BADGE_PROVIDERS,
  badgeProviderHidden,
  clampOverlayStyle,
  type OverlayStyle,
} from './overlayConfig';

const extra = (source: string) => ({ url: `https://example.com/${source}.png`, title: `${source} badge`, source });
const SOURCES = ['ffz', 'moltorino', 'limerino', 'jilchat', 'bchat'];
const template = SAMPLE_MESSAGES.find((m) => (m.provider ?? 'twitch') === 'twitch')!;
const row: OverlayMessage = { ...template, id: 'opt-in', badges: [], extraBadges: SOURCES.map(extra) };

const shown = (style: Partial<OverlayStyle>) => {
  const html = renderToStaticMarkup(<OverlayChat messages={[row]} style={{ ...DEFAULT_OVERLAY_STYLE, badgeLimit: 0, ...style } as OverlayStyle} />);
  return SOURCES.filter((s) => html.includes(`alt="${s} badge"`));
};

describe('opt-in badge providers in the overlay', () => {
  it('names only providers the Services list has', () => {
    const ids = new Set(THIRD_PARTY_BADGE_PROVIDERS.map((p) => p.id));
    for (const id of OPT_IN_BADGE_PROVIDERS) expect(ids.has(id), id).toBe(true);
    expect([...OPT_IN_BADGE_PROVIDERS].sort()).toEqual(['bchat', 'jilchat', 'limerino']);
  });

  it('leaves the new providers off by default and the older ones on', () => {
    expect(shown({})).toEqual(['ffz', 'moltorino']);
  });

  it('keeps them off on a config saved before the setting existed', () => {
    const legacy: Partial<OverlayStyle> = { ...DEFAULT_OVERLAY_STYLE };
    delete legacy.shownBadgeProviders;
    expect(shown(legacy)).toEqual(['ffz', 'moltorino']);
    expect(badgeProviderHidden(legacy as OverlayStyle, 'bchat')).toBe(true);
  });

  it('draws one once switched on, and an old hide entry never turns one on', () => {
    expect(shown({ shownBadgeProviders: ['bchat'] })).toEqual(['ffz', 'moltorino', 'bchat']);
    expect(shown({ hiddenBadgeProviders: ['jilchat', 'ffz'] })).toEqual(['moltorino']);
    expect(badgeProviderHidden({ hiddenBadgeProviders: [], shownBadgeProviders: ['jilchat'] }, 'JilChat')).toBe(false);
  });

  it('keeps only opt-in ids in the shown list', () => {
    const clamped = clampOverlayStyle({ ...DEFAULT_OVERLAY_STYLE, shownBadgeProviders: ['ffz', 'bchat', 'bchat', 'nope'] });
    expect(clamped.shownBadgeProviders).toEqual(['bchat']);
  });
});
