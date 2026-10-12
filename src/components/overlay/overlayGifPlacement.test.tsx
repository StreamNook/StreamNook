// @vitest-environment node
import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { OverlayChat } from './OverlayChat';
import { DEFAULT_OVERLAY_STYLE, OVERLAY_LIMITS, clampOverlayStyle, type OverlayStyle } from './overlayConfig';
import { SAMPLE_MESSAGES } from './sampleMessages';

// A Twitch chat GIF used to be plucked onto a line of its own below the
// message (it shared the giant emote Placement), which left a blank gap above
// it and an indent beside the name. It must flow inline after the name like
// the words of a text message, with its top on the name's line.
const GIF_MSG = SAMPLE_MESSAGES.find((m) => (m.segments ?? []).some((s) => s.type === 'gif'))!;
const GIF_URL = (GIF_MSG.segments ?? []).flatMap((s) => (s.type === 'gif' ? [s.gif_url] : []))[0];

const render = (style: Partial<OverlayStyle>) =>
  renderToStaticMarkup(<OverlayChat messages={[GIF_MSG]} style={{ ...DEFAULT_OVERLAY_STYLE, ...style }} />);

// The <img> tag that carries the GIF (attribute order is React's, so match the tag).
const gifTag = (html: string): string => {
  const escaped = GIF_URL.replace(/&/g, '&amp;');
  const at = html.indexOf(`src="${escaped}"`);
  expect(at).toBeGreaterThan(-1);
  return html.slice(html.lastIndexOf('<img', at), html.indexOf('>', at) + 1);
};

describe('overlay chat GIF placement', () => {
  for (const giantEmoteAlign of ['left', 'center', 'right', 'inline'] as const) {
    it(`draws the GIF inline after the name, top-aligned (giant placement ${giantEmoteAlign})`, () => {
      const html = render({ giantEmoteAlign, showGifs: true });
      const tag = gifTag(html);
      expect(tag).toContain('vertical-align:text-top');
      // Same line block as the name: nothing between the sender's name and the
      // GIF closes the message line, and no plucked flex row was drawn.
      const nameAt = html.indexOf(GIF_MSG.display_name);
      const gifAt = html.indexOf(tag);
      expect(nameAt).toBeGreaterThan(-1);
      expect(gifAt).toBeGreaterThan(nameAt);
      expect(html).not.toContain('display:flex;flex-wrap:wrap;gap:0.25em');
      const lineEnd = html.indexOf('</div>', nameAt);
      expect(gifAt).toBeLessThan(lineEnd);
    });
  }

  it('keeps the GIF inside its size cap', () => {
    const tag = gifTag(render({ showGifs: true, emoteScale: 1 }));
    expect(tag).toContain('height:calc(8em * 1 * 1)');
    expect(tag).toContain('max-width:min(100%, calc(24em * 1 * 1))');
  });

  it('draws the GIF at the GIF size, on top of the emote size', () => {
    const tag = gifTag(render({ showGifs: true, emoteScale: 2, gifScale: 1.5 }));
    expect(tag).toContain('height:calc(8em * 2 * 1.5)');
    expect(tag).toContain('max-width:min(100%, calc(24em * 2 * 1.5))');
    // A bigger GIF keeps the same placement: inline, top on the name's line.
    expect(tag).toContain('vertical-align:text-top');
    const small = gifTag(render({ showGifs: true, emoteScale: 1, gifScale: 0.5 }));
    expect(small).toContain('height:calc(8em * 1 * 0.5)');
  });

  it('gives configs saved before the GIF size today\'s size, and holds it to its range', () => {
    const older: Partial<OverlayStyle> = { ...DEFAULT_OVERLAY_STYLE };
    delete older.gifScale;
    expect(clampOverlayStyle(older as OverlayStyle).gifScale).toBe(1);
    expect(clampOverlayStyle({ ...DEFAULT_OVERLAY_STYLE, gifScale: 9 }).gifScale).toBe(OVERLAY_LIMITS.gifScale.max);
    expect(clampOverlayStyle({ ...DEFAULT_OVERLAY_STYLE, gifScale: 0.1 }).gifScale).toBe(OVERLAY_LIMITS.gifScale.min);
    expect(clampOverlayStyle({ ...DEFAULT_OVERLAY_STYLE, gifScale: Number.NaN }).gifScale).toBe(1);
    // The old renderer drew exactly 8em x emote size; 1 changes nothing.
    expect(DEFAULT_OVERLAY_STYLE.gifScale).toBe(1);
  });

  it('shows the description Twitch sent when GIFs are off', () => {
    const html = render({ showGifs: false });
    expect(html).not.toContain('giphy.gif');
    expect(html).toContain('[Y A Y Yes GIF by Djemilah Birnie]');
  });
});
