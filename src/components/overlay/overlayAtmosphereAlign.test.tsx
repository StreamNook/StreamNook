// @vitest-environment node
// Run with: npm test
//
// An atmosphere paints behind a row without moving it: every side padding the
// wash or its frost adds is paid back with an equal negative margin, so badges,
// name and text start on the same column as a plain row. With bubbles on, the
// wash takes the bubble's padding so its text lines up with theirs.

import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

import { OverlayChat } from './OverlayChat';
import { SAMPLE_MESSAGES, type OverlayAtmosphere, type OverlayMessage } from './sampleMessages';
import { DEFAULT_OVERLAY_STYLE, type OverlayStyle } from './overlayConfig';

const template = SAMPLE_MESSAGES.find(
  (m) => (m.provider ?? 'twitch') === 'twitch' && (m.badges ?? []).length > 0 && !m.metadata?.system_message && !m.tags?.['system-msg'],
)!;
const aurora = SAMPLE_MESSAGES.find((m) => m.atmosphere)!.atmosphere!;

const chatter = (atmosphere: OverlayAtmosphere | null, over: Partial<OverlayMessage> = {}): OverlayMessage => ({
  ...template,
  id: 'atm-row',
  display_name: 'Winters27',
  username: 'winters27',
  tags: {},
  atmosphere,
  ...over,
});

const render = (m: OverlayMessage, style: Partial<OverlayStyle> = {}) =>
  renderToStaticMarkup(<OverlayChat messages={[m]} style={{ ...DEFAULT_OVERLAY_STYLE, entrance: 'none', ...style }} />);

const rowCss = (html: string): string => {
  const at = html.indexOf('data-ov-row=""');
  expect(at).toBeGreaterThan(-1);
  const open = html.lastIndexOf('<div', at);
  return /style="([^"]*)"/.exec(html.slice(open, at))?.[1] ?? '';
};
const frostCss = (html: string): string => {
  const at = html.indexOf('backdrop-filter:blur(8px)');
  expect(at, 'no frost box').toBeGreaterThan(-1);
  return html.slice(html.lastIndexOf('style="', at) + 7, html.indexOf('"', at));
};

// Padding plus margin on one side: zero means the text sits where a plain row's does.
const leftInset = (css: string): number => {
  const side = (prop: string) => {
    const v = new RegExp(`(?:^|;)${prop}:([^;]+)`).exec(css)?.[1].trim().split(/\s+/) ?? [];
    return parseFloat(v[3] ?? v[1] ?? v[0] ?? '0') || 0;
  };
  const longMargin = /(?:^|;)margin-left:(-?[\d.]+)px/.exec(css);
  return side('padding') + (longMargin ? Number(longMargin[1]) : side('margin'));
};

describe('atmosphere rows keep the plain-row column', () => {
  it('bleeds the wash past the text instead of insetting it', () => {
    const css = rowCss(render(chatter(aurora)));
    expect(css).toContain('padding:2px 10px');
    expect(css).toContain('margin-left:-10px');
    expect(css).toContain('margin-right:-10px');
    expect(leftInset(css)).toBe(0);
  });

  it('starts a frosted wash on the same edge and pays back the frost padding too', () => {
    const html = render(chatter({ ...aurora, chatFrost: true }));
    const row = rowCss(html);
    expect(row).toBe(rowCss(render(chatter(aurora))));
    expect(leftInset(row)).toBe(0);
    const frost = frostCss(html);
    expect(frost).toContain('margin:0 -6px');
    expect(frost).toContain('padding:0.5px 6px');
    expect(frost).toContain('box-sizing:border-box');
    expect(leftInset(frost)).toBe(0);
  });

  it('takes a bubble\'s padding when bubbles are on', () => {
    const rounded = rowCss(render(chatter(aurora), { bubble: true }));
    expect(rounded).toContain('padding:0.22em 0.6em');
    expect(rounded).not.toContain('margin-left');
    const pill = rowCss(render(chatter(aurora), { bubble: true, bubbleShape: 'pill' }));
    expect(pill).toContain('padding:0.22em 0.8em');
  });

  it('leaves a first-time row its highlight inset', () => {
    const css = rowCss(render(chatter(aurora, { tags: { 'first-msg': '1' } }), { firstTimeStyle: 'twitch' }));
    expect(css).toContain('padding:3px 8px');
    expect(css).not.toContain('margin-left');
  });

  it('adds nothing when atmospheres are hidden', () => {
    const css = rowCss(render(chatter(aurora), { showAtmospheres: false }));
    expect(css).not.toContain('margin-left');
    expect(css).not.toContain('padding');
  });
});
