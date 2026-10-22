// @vitest-environment node
// Run with: npm test
//
// Badge art resolution, the shadow and what it sits behind, the entrance fill that
// lets finished rows leave the compositor, and typed slider values.

import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

import { OverlayChat } from './OverlayChat';
import { SAMPLE_MESSAGES, type OverlayMessage } from './sampleMessages';
import { hiResBadgeUrl } from './overlayLayout';
import { DEFAULT_OVERLAY_STYLE, clampOverlayStyle, parseSliderInput, type OverlayStyle } from './overlayConfig';

const template = SAMPLE_MESSAGES.find(
  (m) => (m.provider ?? 'twitch') === 'twitch' && (m.badges ?? []).length > 0 && !m.metadata?.system_message && !m.tags?.['system-msg'],
)!;

const chatter = (over: Partial<OverlayMessage> = {}): OverlayMessage => ({
  ...template,
  id: 'shadow-row',
  display_name: 'Winters27',
  username: 'winters27',
  paint: undefined,
  atmosphere: null,
  seventvBadgeUrl: undefined,
  extraBadges: [],
  streamNookUserNumber: null,
  badges: [{ name: 'moderator', version: '1', image_url_1x: 'https://static-cdn.jtvnw.net/badges/v1/abc/1', title: 'Moderator' }],
  ...over,
});

const render = (m: OverlayMessage, style: Partial<OverlayStyle> = {}) =>
  renderToStaticMarkup(<OverlayChat messages={[m]} style={{ ...DEFAULT_OVERLAY_STYLE, entrance: 'none', ...style }} />);

/** The inline style of the element whose text is exactly `text`. */
const styleOf = (html: string, text: string): string => {
  const at = html.indexOf(`>${text}</span>`);
  expect(at, `no element with text ${text}`).toBeGreaterThan(-1);
  const open = html.lastIndexOf('<span', at);
  return /style="([^"]*)"/.exec(html.slice(open, at))?.[1] ?? '';
};
const badgeCluster = (html: string): string => {
  const at = html.indexOf('data-ov-badges=""');
  expect(at).toBeGreaterThan(-1);
  const open = html.lastIndexOf('<span', at);
  return /style="([^"]*)"/.exec(html.slice(open, at))?.[1] ?? '';
};

describe('badge art', () => {
  it('asks each badge CDN for its largest art', () => {
    expect(hiResBadgeUrl('https://static-cdn.jtvnw.net/badges/v1/abc/1')).toBe('https://static-cdn.jtvnw.net/badges/v1/abc/3');
    expect(hiResBadgeUrl('https://static-cdn.jtvnw.net/badges/v1/abc/2')).toBe('https://static-cdn.jtvnw.net/badges/v1/abc/3');
    expect(hiResBadgeUrl('https://static-cdn.jtvnw.net/badges/v1/abc/3')).toBe('https://static-cdn.jtvnw.net/badges/v1/abc/3');
    expect(hiResBadgeUrl('https://cdn.7tv.app/badge/01FNXQY7D00005RKDHEQMRMQN1/1x.webp')).toBe('https://cdn.7tv.app/badge/01FNXQY7D00005RKDHEQMRMQN1/4x.webp');
    expect(hiResBadgeUrl('https://cdn.7tv.app/badge/01FNXQY7D00005RKDHEQMRMQN1/3x')).toBe('https://cdn.7tv.app/badge/01FNXQY7D00005RKDHEQMRMQN1/4x');
    expect(hiResBadgeUrl('https://cdn.7tv.app/badge/01FNXQY7D00005RKDHEQMRMQN1/2x.avif')).toBe('https://cdn.7tv.app/badge/01FNXQY7D00005RKDHEQMRMQN1/4x.webp');
    expect(hiResBadgeUrl('https://cdn.frankerfacez.com/badge/3/1/rounded')).toBe('https://cdn.frankerfacez.com/badge/3/4/rounded');
    expect(hiResBadgeUrl('https://cdn.frankerfacez.com/badge/2/2')).toBe('https://cdn.frankerfacez.com/badge/2/4');
    expect(hiResBadgeUrl('https://cdn.frankerfacez.com/room-badge/mod/id/1/v/a/1')).toBe('https://cdn.frankerfacez.com/room-badge/mod/id/1/v/a/4');
    expect(hiResBadgeUrl('https://yt3.ggpht.com/abcDEF=s16-c-k')).toBe('https://yt3.ggpht.com/abcDEF=s128-c-k');
    expect(hiResBadgeUrl('https://yt3.ggpht.com/abcDEF=w16-h16-c-k-nd')).toBe('https://yt3.ggpht.com/abcDEF=w128-h128-c-k-nd');
  });

  it('leaves art it does not know alone', () => {
    for (const u of [
      'https://fourtf.com/chatterino/badges/supporter3x.png',
      'https://cdn.betterttv.net/badges/developer.svg',
      'https://files.kick.com/channel_subscriber_badges/1/original',
      'https://streamnook.app/cosmetics/streamnook-badge-gold.png',
      'data:image/svg+xml,%3Csvg%3E',
    ]) expect(hiResBadgeUrl(u)).toBe(u);
    expect(hiResBadgeUrl(undefined)).toBeUndefined();
  });

  it('draws a platform badge from its largest art', () => {
    const html = render(chatter());
    // Through StreamNook's badge cache first; the host's own address is the fallback.
    const big = encodeURIComponent('https://static-cdn.jtvnw.net/badges/v1/abc/3');
    expect(html).toContain(`src="https://streamnook.app/api/v1/badge-art?src=${big}"`);
    expect(html).not.toContain('badges/v1/abc/1"');
  });
});

describe('shadow parts', () => {
  const rowCss = (html: string): string => {
    const at = html.indexOf('data-ov-row=""');
    expect(at).toBeGreaterThan(-1);
    const open = html.lastIndexOf('<div', at);
    return /style="([^"]*)"/.exec(html.slice(open, at))?.[1] ?? '';
  };
  const red = { textShadow: true, textShadowColor: '#ff0000', textShadowSize: 2, textShadowOpacity: 1 } as const;
  const soft = 'text-shadow:0 0.067em 0.133em color-mix(in srgb, #ff0000 100%, transparent), 0 0 0.133em color-mix(in srgb, #ff0000 65%, transparent)';
  const painted = () => SAMPLE_MESSAGES.find((m) => m.paint && (m.paint.data.shadows?.length ?? 0) > 0)!;

  it('sits behind text and names by default, not badges', () => {
    const html = render(chatter(), red);
    expect(rowCss(html)).toContain(soft);
    expect(styleOf(html, 'Winters27')).toContain(soft);
    expect(styleOf(html, ':')).toContain(soft);
    expect(badgeCluster(html)).not.toContain('filter');
  });

  it('leaves text out while names keep it', () => {
    const html = render(chatter(), { ...red, shadowOnText: false });
    expect(rowCss(html)).not.toContain('text-shadow');
    expect(styleOf(html, 'Winters27')).toContain(soft);
    expect(styleOf(html, ':')).toContain(soft);
  });

  it('leaves names and their colon out while text keeps it', () => {
    const html = render(chatter(), { ...red, shadowOnNames: false });
    expect(rowCss(html)).toContain(soft);
    expect(styleOf(html, 'Winters27')).toContain('text-shadow:none');
    expect(styleOf(html, ':')).toContain('text-shadow:none');
  });

  it('draws one soft drop-shadow on the whole badge cluster when badges are picked', () => {
    const css = badgeCluster(render(chatter(), { ...red, textShadowSize: 4, textShadowOpacity: 0.5, shadowOnBadges: true }));
    expect(css).toContain('filter:drop-shadow(0 0.067em 0.267em color-mix(in srgb, #ff0000 50%, transparent))');
    expect(css.match(/drop-shadow\(/g)).toHaveLength(1);
  });

  it('draws nothing anywhere with the shadow off or at size 0', () => {
    for (const off of [{ textShadow: false }, { textShadowSize: 0 }]) {
      const html = render(chatter(), { ...red, ...off, shadowOnBadges: true });
      expect(rowCss(html)).not.toContain('color-mix');
      expect(badgeCluster(html)).not.toContain('filter');
      expect(styleOf(html, 'Winters27')).not.toContain('color-mix');
    }
  });

  it('casts a painted name through a filter after the paint\'s own shadows, and not when names are left out', () => {
    const css = styleOf(render(chatter({ paint: painted().paint }), red), 'Winters27');
    expect(css).toContain('background-clip:text');
    expect(css).toContain('text-shadow:none');
    expect(css).toMatch(/filter:drop-shadow\(#[0-9a-f]+ \d+px \d+px \d+px\)( drop-shadow\([^)]*\))* drop-shadow\(0 0\.033em 0\.133em color-mix\(in srgb, #ff0000 100%, transparent\)\)(;|$)/i);
    const out = styleOf(render(chatter({ paint: painted().paint }), { ...red, shadowOnNames: false }), 'Winters27');
    expect(out).not.toContain('color-mix');
  });

  it('turns dark for each part on its own', () => {
    const contour = '0 0 0.067em color-mix(in srgb, #ff0000 100%, transparent)';
    const stacked = '0 0.067em 0.189em color-mix(in srgb, #ff0000 65%, transparent)';
    const darkText = render(chatter(), { ...red, shadowOnBadges: true, shadowDarkText: true });
    expect(rowCss(darkText)).toContain(`text-shadow:${contour}, 0 0.067em 0.133em color-mix(in srgb, #ff0000 100%, transparent), 0 0 0.133em color-mix(in srgb, #ff0000 65%, transparent), ${stacked}`);
    expect(styleOf(darkText, 'Winters27')).not.toContain(stacked);
    expect(badgeCluster(darkText)).not.toContain(contour);

    const darkNames = render(chatter(), { ...red, shadowDarkNames: true });
    expect(styleOf(darkNames, 'Winters27')).toContain(stacked);
    expect(styleOf(darkNames, ':')).toContain(stacked);
    expect(rowCss(darkNames)).not.toContain(stacked);

    const darkBadges = badgeCluster(render(chatter(), { ...red, shadowOnBadges: true, shadowDarkBadges: true }));
    expect(darkBadges).toContain(`filter:drop-shadow(${contour}) drop-shadow(0 0.067em 0.133em color-mix(in srgb, #ff0000 100%, transparent)) drop-shadow(0 0 0.133em color-mix(in srgb, #ff0000 65%, transparent))`);

    const darkPaint = styleOf(render(chatter({ paint: painted().paint }), { ...red, shadowDarkNames: true }), 'Winters27');
    expect(darkPaint).toContain(`drop-shadow(${contour})`);
    expect(darkPaint).not.toContain('0.189em');
  });

  it('lets a plain row draw the shadow past its edges', () => {
    const css = rowCss(render(chatter(), red));
    expect(css).toContain('overflow:visible');
    expect(css).not.toContain('margin-left');
  });

  it('gives the line cap room on the sides and the top', () => {
    // Size 2 at the 15 px default font: (2 * 1.5 + 1) / 15 = 0.267em.
    const html = render(chatter(), { ...red, maxMessageLines: 2 });
    const at = html.indexOf('-webkit-line-clamp:2');
    expect(at).toBeGreaterThan(-1);
    const css = html.slice(html.lastIndexOf('style="', at), html.indexOf('"', at));
    for (const side of ['left', 'right', 'top']) {
      expect(css).toContain(`padding-${side}:0.267em`);
      expect(css).toContain(`margin-${side}:-0.267em`);
    }
    expect(css).not.toContain('padding-bottom');
  });

  it('casts the YouTube owner pill shadow from the pill when badges are picked', () => {
    const yt = chatter({ provider: 'youtube', badges: [{ name: 'broadcaster', version: '1', title: 'Owner' }] });
    const css = styleOf(render(yt, { ...red, shadowOnBadges: true }), 'Winters27');
    expect(css).toContain('background-color:#ffd600');
    expect(css).toContain('filter:drop-shadow(');
    expect(css).toContain('text-shadow:none');
    expect(styleOf(render(yt, red), 'Winters27')).not.toContain('filter:drop-shadow(');
  });
});

describe('configs saved before the shadow parts', () => {
  const legacy = (over: Record<string, unknown>) => {
    const base: Record<string, unknown> = { ...DEFAULT_OVERLAY_STYLE };
    for (const k of ['shadowOnText', 'shadowOnNames', 'shadowOnBadges', 'shadowDarkText', 'shadowDarkNames', 'shadowDarkBadges']) delete base[k];
    return clampOverlayStyle({ ...base, ...over } as unknown as OverlayStyle);
  };

  it('keep drawing behind text and names, with no badges', () => {
    const s = legacy({});
    expect([s.textShadow, s.shadowOnText, s.shadowOnNames, s.shadowOnBadges]).toEqual([true, true, true, false]);
  });

  it('carry the old badge switch over and drop the retired keys', () => {
    const s = legacy({ badgeShadow: true, badgeShadowColor: '#123456', nameShadow: true, nameShadowStyle: 'thick' });
    expect(s.shadowOnBadges).toBe(true);
    expect(s.shadowOnNames).toBe(true);
    for (const k of ['badgeShadow', 'badgeShadowColor', 'nameShadow', 'nameShadowStyle']) expect(s).not.toHaveProperty(k);
  });

  it('carry the old Thick choice over to dark text and names', () => {
    const s = legacy({ textShadowStyle: 'thick' });
    expect([s.shadowDarkText, s.shadowDarkNames, s.shadowDarkBadges]).toEqual([true, true, false]);
    expect(s).not.toHaveProperty('textShadowStyle');
    const soft = legacy({ textShadowStyle: 'soft' });
    expect([soft.shadowDarkText, soft.shadowDarkNames]).toEqual([false, false]);
  });

  it('turn the shadow on for just the parts that had their own with the text shadow off', () => {
    const s = legacy({ textShadow: false, nameShadow: true });
    expect([s.textShadow, s.shadowOnText, s.shadowOnNames, s.shadowOnBadges]).toEqual([true, false, true, false]);
    const off = legacy({ textShadow: false });
    expect([off.textShadow, off.shadowOnText, off.shadowOnNames]).toEqual([false, true, true]);
  });

  it('are left alone once they have the switches', () => {
    const s = clampOverlayStyle({ ...DEFAULT_OVERLAY_STYLE, shadowOnText: false, shadowOnNames: false, shadowOnBadges: true });
    expect([s.shadowOnText, s.shadowOnNames, s.shadowOnBadges]).toEqual([false, false, true]);
  });
});

describe('entrance animations', () => {
  it('fill backwards so a finished row leaves the compositor', () => {
    const html = render(chatter(), { entrance: 'fade' });
    for (const cls of ['fade', 'slide', 'drift', 'rise', 'pop', 'stamp']) {
      const rule = new RegExp(`\\.sn-ov-${cls} \\{ animation: [^;]+ (\\w+);`).exec(html);
      expect(rule?.[1], cls).toBe('backwards');
    }
    // The retract fade still holds its end state until the row is removed.
    expect(html).toMatch(/\.sn-ov-out \{ animation: snOvOut 260ms ease both;/);
  });
});

describe('typed slider values', () => {
  const px = { min: 10, max: 48, step: 1 };
  it('takes a bare number or one with its unit', () => {
    expect(parseSliderInput('14', px)).toBe(14);
    expect(parseSliderInput(' 22px ', px)).toBe(22);
  });
  it('clamps to the range and snaps to the step', () => {
    expect(parseSliderInput('4', px)).toBe(10);
    expect(parseSliderInput('99', px)).toBe(48);
    expect(parseSliderInput('14.6', px)).toBe(15);
    expect(parseSliderInput('1.43', { min: 1, max: 2.2, step: 0.05 })).toBe(1.45);
    expect(parseSliderInput('237', { min: 160, max: 900, step: 10 })).toBe(240);
  });
  it('reads a percent field back into a 0 to 1 value', () => {
    const pct = { min: 0, max: 1, step: 0.05, scale: 100 };
    expect(parseSliderInput('85', pct)).toBe(0.85);
    expect(parseSliderInput('85%', pct)).toBe(0.85);
    expect(parseSliderInput('33', pct)).toBe(0.35);
    expect(parseSliderInput('150', pct)).toBe(1);
  });
  it('accepts a comma decimal and rejects what is not a number', () => {
    expect(parseSliderInput('1,5', { min: 0, max: 12, step: 0.5 })).toBe(1.5);
    expect(parseSliderInput('', px)).toBeNull();
    expect(parseSliderInput('abc', px)).toBeNull();
  });
});
