// The song link's QR code: encoded at level H for every link length, the
// logo's square centered and small enough for level H to cover, clear of the
// finder patterns, the timing rows and the format areas, and every other dark
// module drawn as one rounded square of its own.
import { describe, expect, it } from 'vitest';
import { QrCodeDataType } from 'uqr';
import { inBox, KNOCKOUT_SHARE, qrHeading, qrModel, shortLink, type QrModel } from './songQr';

const LONG_APPLE =
  'https://music.apple.com/us/album/debi-tirar-mas-fotos-collectors-edition/1787022393?i=1787022572&l=en-US';
const SHORT_DEEZER = 'https://www.deezer.com/track/3112345678';

/** The error correction level the code says it uses, read back from its
 *  format bits (the copy beside the top-left finder, unmasked). */
function eccOf({ dark }: QrModel): 'L' | 'M' | 'Q' | 'H' {
  const at = (x: number, y: number) => (dark[y][x] ? 1 : 0);
  const bit = (i: number) =>
    i <= 5 ? at(8, i) : i === 6 ? at(8, 7) : i === 7 ? at(8, 8) : i === 8 ? at(7, 8) : at(14 - i, 8);
  let bits = 0;
  for (let i = 0; i < 15; i++) bits |= bit(i) << i;
  const level = ((bits ^ 0x5412) >> 10) >> 3;
  return (['M', 'L', 'H', 'Q'] as const)[level];
}

const KEPT = new Set([QrCodeDataType.Position, QrCodeDataType.Timing, QrCodeDataType.Function]);

describe('the song link’s QR code', () => {
  it('is encoded at level H, long link or short', () => {
    const long = qrModel(LONG_APPLE);
    const short = qrModel(SHORT_DEEZER);
    expect(eccOf(long)).toBe('H');
    expect(eccOf(short)).toBe('H');
    // A longer link needs a larger code at the same level.
    expect(long.size).toBeGreaterThan(short.size);
  });

  it('clears a centered square for the logo, at most 22% of the side, touching no finder, timing or format module', () => {
    for (const url of [LONG_APPLE, SHORT_DEEZER, 'https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT']) {
      const { size, knockout, logo, types } = qrModel(url);
      expect(knockout.size, url).toBeGreaterThanOrEqual(3);
      expect(knockout.size / size, url).toBeLessThanOrEqual(KNOCKOUT_SHARE);
      // Centered on the middle module.
      expect(knockout.x, url).toBe((size - knockout.size) / 2);
      expect(knockout.y, url).toBe(knockout.x);
      for (let y = 0; y < size; y++) {
        for (let x = 0; x < size; x++) {
          if (inBox(knockout, x, y)) expect(KEPT.has(types[y][x]), `${url} (${x}, ${y})`).toBe(false);
        }
      }
      // The logo is one module narrower than its white backing.
      expect(logo.size).toBe(knockout.size - 1);
      expect(logo.x).toBe(knockout.x + 0.5);
    }
  });

  it('draws one rounded square per dark module outside the finders and the knockout', () => {
    const model = qrModel(LONG_APPLE);
    let expected = 0;
    for (let y = 0; y < model.size; y++) {
      for (let x = 0; x < model.size; x++) {
        if (model.dark[y][x] && model.types[y][x] !== QrCodeDataType.Position && !inBox(model.knockout, x, y)) {
          expected++;
        }
      }
    }
    const squares = model.modules.match(/M/g) ?? [];
    expect(squares.length).toBe(expected);
    // Each one rounded at all four corners.
    expect(model.modules.match(/a0\.35 0\.35/g)?.length).toBe(expected * 4);
    // The finders: three rings, three holes, three dots.
    expect(model.finders.match(/M/g)?.length).toBe(9);
  });
});

describe('the dialog’s words', () => {
  it('names the service, and a search as one', () => {
    expect(qrHeading({ service: 'Spotify', search: false })).toBe('Open in Spotify on your phone');
    expect(qrHeading({ service: 'Spotify', search: true })).toBe('Search Spotify on your phone');
  });

  it('shortens a link to its host and path', () => {
    expect(shortLink('https://www.deezer.com/track/3112345678')).toBe('deezer.com/track/3112345678');
    expect(shortLink('https://open.spotify.com/search/Bad%20Bunny?si=1')).toBe('open.spotify.com/search/Bad Bunny');
    expect(shortLink('https://music.apple.com/us/album/x/1?i=2')).toBe('music.apple.com/us/album/x/1');
    expect(shortLink('not a link')).toBe('not a link');
  });
});
