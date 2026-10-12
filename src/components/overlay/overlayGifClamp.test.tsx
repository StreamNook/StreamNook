// @vitest-environment node
// Run with: npm test
//
// Max lines holds for GIF rows too. A GIF sits inline in the clamped line
// block, top-aligned, so the clamp counts its line as one and hides whole lines
// after the cap: a Tier 2/3 GIF can no longer carry a long message past it.

import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

import { OverlayChat } from './OverlayChat';
import { SAMPLE_MESSAGES, type OverlayMessage } from './sampleMessages';
import { DEFAULT_OVERLAY_STYLE, GIANT_EMOTE_ALIGNS, type OverlayStyle } from './overlayConfig';
import { MAX_MESSAGE_LINES_CAP, gifLineIndex, messageLineClamp } from './overlayLayout';

const GIF_MSG = SAMPLE_MESSAGES.find((m) => (m.segments ?? []).some((s) => s.type === 'gif'))!;
const LONG_GIF_MSG: OverlayMessage = {
  ...GIF_MSG,
  id: 'gif-copypasta',
  segments: [...(GIF_MSG.segments ?? []), { type: 'text', content: ` ${'copypasta '.repeat(45)}` } as NonNullable<OverlayMessage['segments']>[number]],
};

const render = (style: Partial<OverlayStyle>) =>
  renderToStaticMarkup(<OverlayChat messages={[LONG_GIF_MSG]} style={{ ...DEFAULT_OVERLAY_STYLE, ...style }} />);

describe('Max lines on a GIF row', () => {
  for (const { value: giantEmoteAlign } of GIANT_EMOTE_ALIGNS) {
    it(`clamps the row with the GIF in it (giant placement ${giantEmoteAlign})`, () => {
      const html = render({ maxMessageLines: 2, giantEmoteAlign });
      expect(html).toContain('-webkit-line-clamp:2');
      // The GIF stays inside the clamped line block, inline after the name.
      const clampAt = html.indexOf('-webkit-line-clamp:2');
      const gifAt = html.indexOf('giphy.gif');
      expect(gifAt).toBeGreaterThan(clampAt);
      expect(html.indexOf('</div>', clampAt)).toBeGreaterThan(gifAt);
    });
  }

  it('leaves a GIF row unclamped when Max lines is off', () => {
    expect(render({ maxMessageLines: 0 })).not.toContain('-webkit-line-clamp');
  });

  it('still draws the GIF when GIFs are on and the row is clamped', () => {
    const html = render({ maxMessageLines: 1, showGifs: true });
    expect(html).toContain('vertical-align:text-top');
  });
});

// A GIF too wide for what is left of the name's line wraps below it (a 400px
// overlay at GIF size 1.50x). A plain cap of one line then cut the GIF's whole
// line away and the GIF vanished. The row measures the line the GIF starts on
// and the cap always reaches it: the GIF's line counts as one line and the
// text clamps around it.
describe('Max lines with a GIF wider than the rest of the name line', () => {
  // Boxes as the browser reports them for everything before the GIF, at a 16px
  // font: the badge (middle-aligned), the name, and "FINALLY" on line 0.
  const nameLine = [
    { top: 2, bottom: 20 },
    { top: 0, bottom: 19 },
    { top: 0, bottom: 19 },
  ];

  it('finds the GIF on the line below the name when it wrapped', () => {
    // Line 0 ends at 22; a wrapped GIF hangs from the top of line 1.
    expect(gifLineIndex(nameLine, 25)).toBe(1);
  });

  it('keeps the GIF on the name line when it fits there', () => {
    // Text-top: the GIF's top is the top of the name's text, and everything on
    // that line has its middle below it.
    expect(gifLineIndex(nameLine, 0)).toBe(0);
    expect(gifLineIndex([], 0)).toBe(0);
  });

  it('counts lines above it once each, even when tight line height overlaps them', () => {
    // Line height 1.0: the second line's text starts above the first one's end.
    const twoLines = [...nameLine, { top: 16, bottom: 35 }, { top: 17, bottom: 36 }];
    expect(gifLineIndex(twoLines, 32)).toBe(2);
  });

  it("raises a cap of one to reach the GIF's line, and leaves a cap that already does", () => {
    expect(messageLineClamp(1, 1)).toBe(2);
    expect(messageLineClamp(2, 1)).toBe(2);
    expect(messageLineClamp(3, 1)).toBe(3);
    expect(messageLineClamp(1, 0)).toBe(1);
  });

  it('leaves Max lines off and keeps its own ceiling for text', () => {
    expect(messageLineClamp(0, 1)).toBe(0);
    expect(messageLineClamp(undefined, 3)).toBe(0);
    expect(messageLineClamp(9, 0)).toBe(MAX_MESSAGE_LINES_CAP);
  });

  it('marks the GIF so the row can find it, and renders the measured row clamped', () => {
    const html = render({ maxMessageLines: 1, showGifs: true, gifScale: 1.5 });
    const gifAt = html.indexOf('giphy.gif');
    const tag = html.slice(html.lastIndexOf('<img', gifAt), html.indexOf('>', gifAt) + 1);
    expect(tag).toContain('data-overlay-gif=""');
    // Before the browser has measured anything, the cap is the setting's.
    expect(html).toContain('-webkit-line-clamp:1');
  });
});
