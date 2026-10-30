// The QR dialog's card and the song page's link buttons, drawn on the server
// (no DOM here): the card names the service and offers Copy link, Close and
// every way to open the link on this computer; the desktop's pills carry a QR
// part and the search logos a QR button; a phone draws none of them.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

const platform = vi.hoisted(() => ({ mobile: false }));
vi.mock('../../utils/platform', async (importOriginal) => {
  const real = await importOriginal<typeof import('../../utils/platform')>();
  return {
    ...real,
    get IS_MOBILE() {
      return platform.mobile;
    },
  };
});
vi.mock('../../stores/AppStore', () => ({ useAppStore: { getState: () => ({ addToast: vi.fn() }) } }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

import { SongQrCard } from './SongQrDialog';
import { SongLinkButtons } from './SongLinkButtons';
import type { SongApp } from '../../utils/songLinks';

const cider: SongApp = { service: 'Apple Music', app: 'Cider', target: 'CiderCollective.Cider_a6qxe093bx5xj!App', default: true };
const apple: SongApp = { service: 'Apple Music', app: 'Apple Music', target: 'AppleInc.AppleMusicWin_x!App', default: false };

const links = {
  listen: [
    { name: 'Spotify', url: 'https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT' },
    { name: 'Apple Music', url: 'https://music.apple.com/us/song/950742158' },
  ],
  search: [{ name: 'Tidal', url: 'https://tidal.com/search?q=Sade', search: true }],
  allServices: null,
};

const card = (apps: SongApp[] = []) =>
  renderToStaticMarkup(
    <SongQrCard
      request={{ url: 'https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT', service: 'Spotify', search: false }}
      title="Kiss of Life"
      artist="Sade"
      apps={apps}
      onClose={() => {}}
      onOpen={() => {}}
    />,
  );

describe('the QR dialog', () => {
  it('names the service, shows the code and offers Copy link and Close', () => {
    const html = card();
    expect(html).toContain('Open in Spotify on your phone');
    expect(html).toContain('Kiss of Life');
    expect(html).toContain('Copy link');
    expect(html).toContain('aria-label="Close"');
    expect(html).toContain('aria-label="QR code"');
    expect(html).toContain('open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT');
    expect(html).toContain("Point your phone&#x27;s camera at the code");
  });

  it('still opens and copies a link too long for a code, saying so where the code would be', () => {
    const url = `https://open.spotify.com/search/${'%E3%81%82'.repeat(260)}`;
    const html = renderToStaticMarkup(
      <SongQrCard
        request={{ url, service: 'Spotify', search: true }}
        title="Kiss of Life"
        artist="Sade"
        apps={[]}
        onClose={() => {}}
        onOpen={() => {}}
      />,
    );
    expect(html).toContain('This link is too long for a QR code');
    expect(html).not.toContain('aria-label="QR code"');
    expect(html).not.toContain('camera');
    expect(html).toContain('Copy link');
    expect(html).toContain('Open in your browser');
  });

  it('offers each app on this computer, then the browser', () => {
    const html = card([cider, apple]);
    expect(html).toContain('Open in Cider');
    expect(html).toContain('Open in Apple Music');
    expect(html.indexOf('Open in Cider')).toBeLessThan(html.indexOf('Open in Apple Music'));
    expect(html).toContain('Open in your browser');
    // With no app here, only the browser.
    expect(card()).not.toContain('Open in Cider');
    expect(card()).toContain('Open in your browser');
  });
});

describe('the song page’s link buttons', () => {
  beforeEach(() => {
    platform.mobile = false;
  });

  const buttons = (screen = false) =>
    renderToStaticMarkup(<SongLinkButtons links={links} title="Kiss of Life" artist="Sade" screen={screen} />);

  it('carry a QR part on every pill and a QR button by every search logo on the desktop', () => {
    const html = buttons();
    expect(html).toContain('aria-label="Show a QR code for Spotify"');
    expect(html).toContain('aria-label="Show a QR code for Apple Music"');
    expect(html).toContain('aria-label="Show a QR code for the Tidal search"');
  });

  it('draw no QR button for a link too long for any code', () => {
    const long = `https://open.spotify.com/search/${'%E3%81%82'.repeat(260)}`;
    const html = renderToStaticMarkup(
      <SongLinkButtons
        links={{
          listen: [{ name: 'Spotify', url: long }],
          search: [{ name: 'Tidal', url: `https://tidal.com/search?q=${'%E3%81%82'.repeat(260)}`, search: true }],
          allServices: null,
        }}
        title="Kiss of Life"
        artist="Sade"
        screen={false}
      />,
    );
    expect(html).toContain('Spotify');
    expect(html).toContain('aria-label="Search on Tidal"');
    expect(html).not.toContain('QR code');
  });

  it('draw no QR controls on a phone', () => {
    platform.mobile = true;
    const html = buttons();
    expect(html).toContain('Spotify');
    expect(html).toContain('aria-label="Search on Tidal"');
    expect(html).not.toContain('QR code');
    expect(buttons(true)).not.toContain('QR code');
  });
});
