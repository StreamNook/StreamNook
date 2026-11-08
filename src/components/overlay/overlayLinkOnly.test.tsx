// @vitest-environment node
// Run with: npm test
//
// Hide link-only messages: a message that is just a link stays off the overlay,
// while a link sent with words or an emote beside it still shows.

import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

import { OverlayChat } from './OverlayChat';
import { isLinkOnlyMessage } from './overlayLayout';
import { SAMPLE_MESSAGES, type OverlayMessage } from './sampleMessages';
import { DEFAULT_OVERLAY_STYLE, type OverlayStyle } from './overlayConfig';

type Segment = NonNullable<OverlayMessage['segments']>[number];

const text = (content: string) => ({ type: 'text', content }) as Segment;
const link = (content: string) => ({ type: 'link', content, url: `https://${content}` }) as Segment;
const emote = { type: 'emote', content: 'Kappa', emote_id: '25', emote_url: 'https://e/kappa.png' } as unknown as Segment;

const BASE = SAMPLE_MESSAGES.find((m) => (m.segments ?? []).every((s) => s.type === 'text'))!;
const msg = (id: string, segments: Segment[]): OverlayMessage => ({
  ...BASE,
  id,
  content: segments.map((s) => s.content).join(''),
  segments,
});

describe('isLinkOnlyMessage', () => {
  it('is true for a bare link, several links, and links with only punctuation', () => {
    expect(isLinkOnlyMessage([link('test.fr')])).toBe(true);
    expect(isLinkOnlyMessage([text('  '), link('a.com'), text(' '), link('b.com'), text(' ')])).toBe(true);
    expect(isLinkOnlyMessage([link('a.com'), text(' !!')])).toBe(true);
    expect(isLinkOnlyMessage([text('-> '), link('a.com')])).toBe(true);
  });

  it('is false once any word, number or emote sits beside the link', () => {
    expect(isLinkOnlyMessage([text('bla bla bla message '), link('a.com')])).toBe(false);
    expect(isLinkOnlyMessage([link('a.com'), text(' 2')])).toBe(false);
    expect(isLinkOnlyMessage([link('a.com'), text(' '), emote])).toBe(false);
    expect(isLinkOnlyMessage([link('a.com'), text(' été')])).toBe(false);
  });

  it('counts a link drawn as a picture as content, not a bare link', () => {
    const picture = (url: string) => url.endsWith('.png');
    expect(isLinkOnlyMessage([link('i.nuuls.com/cat.png')], picture)).toBe(false);
    expect(isLinkOnlyMessage([link('i.nuuls.com/cat.png'), text(' '), link('a.com')], picture)).toBe(false);
    // Without the picture rule (images off) the same message is just a link.
    expect(isLinkOnlyMessage([link('i.nuuls.com/cat.png')])).toBe(true);
  });

  it('is false with no link, and with no segments at all', () => {
    expect(isLinkOnlyMessage([text('hello')])).toBe(false);
    expect(isLinkOnlyMessage([text('  ')])).toBe(false);
    expect(isLinkOnlyMessage([])).toBe(false);
    expect(isLinkOnlyMessage(undefined)).toBe(false);
  });
});

describe('Hide link-only messages on the overlay', () => {
  const bare = msg('bare-link', [link('bareonly.example')]);
  const said = msg('said-link', [text('look at this '), link('withwords.example')]);
  const render = (style: Partial<OverlayStyle>) =>
    renderToStaticMarkup(<OverlayChat messages={[bare, said]} style={{ ...DEFAULT_OVERLAY_STYLE, ...style }} />);

  it('shows both messages while the switch is off (the default)', () => {
    const html = render({});
    expect(html).toContain('bareonly.example');
    expect(html).toContain('withwords.example');
  });

  it('hides only the bare link when the switch is on', () => {
    const html = render({ hideLinkOnly: true });
    expect(html).not.toContain('bareonly.example');
    expect(html).toContain('withwords.example');
  });

  const picture = msg('picture-link', [link('i.nuuls.com/cat.png')]);
  const renderPicture = (style: Partial<OverlayStyle>) =>
    renderToStaticMarkup(<OverlayChat messages={[picture]} style={{ ...DEFAULT_OVERLAY_STYLE, hideLinkOnly: true, ...style }} />);

  it('keeps a picture-only message the overlay draws as a picture', () => {
    const html = renderPicture({ showUploadedImages: true, uploadedImagesFrom: 'everyone' });
    expect(html).toContain('src="https://i.nuuls.com/cat.png"');
  });

  it('hides the same message when uploaded pictures are off, since it shows as a bare link', () => {
    expect(renderPicture({ showUploadedImages: false })).not.toContain('i.nuuls.com/cat.png');
  });
});
