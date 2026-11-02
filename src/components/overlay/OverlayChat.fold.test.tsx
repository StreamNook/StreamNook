// @vitest-environment node
// Run with: npm test
//
// The overlay folds a chatter's badges past its "Badges before +N" limit into
// a static peek pill: the next two hidden badges' art, then the count.

import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

import { OverlayChat } from './OverlayChat';
import { SAMPLE_MESSAGES, type OverlayMessage } from './sampleMessages';
import { DEFAULT_OVERLAY_STYLE, clampOverlayStyle, type OverlayStyle } from './overlayConfig';

const template = SAMPLE_MESSAGES.find(
  (m) => (m.provider ?? 'twitch') === 'twitch' && (m.badges ?? []).length > 0 && !m.metadata?.system_message && !m.tags?.['system-msg'],
)!;

const tw = (name: string, id: string, title: string) => ({
  name,
  version: '1',
  image_url_1x: `https://static-cdn.jtvnw.net/badges/v1/${id}/1`,
  title,
});

// Seven badges in chat order: three Twitch (the moderator a role), 7TV, three add-ons.
const crowded = (): OverlayMessage => ({
  ...template,
  id: 'fold-row',
  display_name: 'Zo0x_',
  username: 'zo0x_',
  paint: undefined,
  atmosphere: null,
  streamNookUserNumber: null,
  badges: [tw('moderator', 'mod', 'Moderator'), tw('subscriber', 'sub', 'Subscriber'), tw('bits', 'bits', 'Bits')],
  seventvBadgeUrl: 'https://cdn.7tv.app/badge/01FOLDTEST/1x.webp',
  seventvBadgeTitle: '7TV Subscriber',
  extraBadges: [
    { url: 'https://cdn.frankerfacez.com/badge/3/2', title: 'FFZ Supporter', source: 'ffz' },
    { url: 'https://fourtf.com/chatterino/badges/supporter2x.png', title: 'Chatterino Supporter', source: 'chatterino' },
    { url: 'https://api.dank.chat/badges/dank.png', title: 'DankChat Supporter', source: 'dankchat' },
  ],
});

const render = (style: Partial<OverlayStyle>) =>
  renderToStaticMarkup(<OverlayChat messages={[crowded()]} style={{ ...DEFAULT_OVERLAY_STYLE, entrance: 'none', ...style }} />);

/** The badge cluster's markup (up to the name), split into the kept part and the fold pill. */
const cluster = (html: string) => {
  const start = html.indexOf('data-ov-badges=""');
  expect(start).toBeGreaterThan(-1);
  const body = html.slice(start, html.indexOf('Zo0x_', start));
  const foldAt = body.indexOf('data-ov-badge-fold=""');
  return { kept: foldAt === -1 ? body : body.slice(0, foldAt), fold: foldAt === -1 ? '' : body.slice(foldAt) };
};
const imgs = (html: string) => (html.match(/<img /g) ?? []).length;

describe('overlay badge fold', () => {
  it('keeps three badges and folds the rest into a peek pill', () => {
    const { kept, fold } = cluster(render({ badgeLimit: 3 }));
    expect(imgs(kept)).toBe(3);
    expect(fold).not.toBe('');
    expect(imgs(fold)).toBe(2);
    expect(fold).toContain('+4');
  });

  it('shows every badge with the limit off', () => {
    const html = render({ badgeLimit: 0 });
    expect(html).not.toContain('data-ov-badge-fold');
    expect(imgs(cluster(html).kept)).toBe(7);
  });

  it('clamps a stored limit and defaults a missing one to 3', () => {
    expect(clampOverlayStyle({ ...DEFAULT_OVERLAY_STYLE, badgeLimit: 1 }).badgeLimit).toBe(2);
    expect(clampOverlayStyle({ ...DEFAULT_OVERLAY_STYLE, badgeLimit: 9 }).badgeLimit).toBe(5);
    const legacy: Record<string, unknown> = { ...DEFAULT_OVERLAY_STYLE };
    delete legacy.badgeLimit;
    expect(clampOverlayStyle(legacy as unknown as OverlayStyle).badgeLimit).toBe(3);
  });
});
