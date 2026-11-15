// @vitest-environment node
// Run with: npm test
//
// A badge image draws Twitch's own file until Rust has the sharp copy on disk,
// then the disk copy; the CDN URL itself is never put in `src`. A disk copy that
// fails to load sends the image back to Twitch for the session. A server render
// is the first paint, before any effect could run.

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, describe, expect, it, vi } from 'vitest';

const art = vi.hoisted(() => ({ copies: new Map<string, string>(), requested: [] as string[] }));
vi.mock('../../services/streamnookArt', () => ({
  streamnookArtCopyOf: (url: string) => art.copies.get(url) ?? null,
  requestStreamnookArt: (url: string) => {
    art.requested.push(url);
  },
  subscribeStreamnookArt: () => () => {},
  getStreamnookArtVersion: () => 0,
}));

import { SharpBadgeImg, _failedSharpCopies, sharpSrc } from './SharpBadgeImg';

const L = '34d2d3cad154dbffb1ce314ce9cbbaf767dccbfdce7c9acc739edb19eac42401';
const S = '9801d687fca7d8744e77290e289a35535eeea91bac87bf28e3039d580ea7d605';
const LARGE = `https://cdn.streamnook.app/twitch-badges/${L}.png`;
const SMALL = `https://cdn.streamnook.app/twitch-badges/${S}.webp`;
const TWITCH = 'https://static-cdn.jtvnw.net/badges/v1/abc/3';
const COPY = 'http://asset.localhost/C%3A/cache/large.png';
const badgeArt = { i: TWITCH, a: `${L}.${S}` };

const render = (props: Partial<Parameters<typeof SharpBadgeImg>[0]> = {}) =>
  renderToStaticMarkup(createElement(SharpBadgeImg, { src: TWITCH, art: badgeArt, cssPx: 160, alt: 'Wolf', ...props }));

afterEach(() => {
  art.copies.clear();
  art.requested.length = 0;
  _failedSharpCopies.clear();
  vi.unstubAllGlobals();
});

describe('sharpSrc', () => {
  it('draws Twitch when nothing is picked or there is no copy', () => {
    expect(sharpSrc(TWITCH, null, COPY, new Set())).toBe(TWITCH);
    expect(sharpSrc(TWITCH, LARGE, null, new Set())).toBe(TWITCH);
  });

  it('draws the disk copy when there is one', () => {
    expect(sharpSrc(TWITCH, LARGE, COPY, new Set())).toBe(COPY);
  });

  it('draws Twitch, never the failed copy, once that copy failed', () => {
    expect(sharpSrc(TWITCH, LARGE, COPY, new Set([COPY]))).toBe(TWITCH);
  });
});

describe('SharpBadgeImg', () => {
  it('draws Twitch with no disk copy, and the CDN URL is never in the markup', () => {
    const html = render();
    expect(html).toContain(`src="${TWITCH}"`);
    expect(html).not.toContain('cdn.streamnook.app');
  });

  it('draws the disk copy once Rust has one', () => {
    art.copies.set(LARGE, COPY);
    expect(render()).toContain(`src="${COPY}"`);
  });

  it('goes back to Twitch after the copy failed this session', () => {
    art.copies.set(LARGE, COPY);
    _failedSharpCopies.add(COPY);
    const html = render();
    expect(html).toContain(`src="${TWITCH}"`);
    expect(html).not.toContain(COPY);
  });

  it('picks the file by the screen ratio', () => {
    art.copies.set(SMALL, 'http://asset.localhost/small.webp');
    expect(render({ cssPx: 64 })).toContain(`src="${TWITCH}"`);
    vi.stubGlobal('window', { devicePixelRatio: 2 });
    expect(render({ cssPx: 64 })).toContain('src="http://asset.localhost/small.webp"');
  });

  it('keeps the old image and its attributes without sharp art', () => {
    const html = render({
      art: undefined,
      className: 'w-16 h-16 object-contain',
      loading: 'lazy',
      decoding: 'async',
      draggable: false,
    });
    expect(html).toBe(
      `<img src="${TWITCH}" alt="Wolf" class="w-16 h-16 object-contain" loading="lazy" decoding="async" draggable="false"/>`,
    );
  });

  it('draws Twitch beside art made from another image', () => {
    art.copies.set(LARGE, COPY);
    expect(render({ art: { i: 'https://static-cdn.jtvnw.net/badges/v1/other/3', a: badgeArt.a } })).toContain(
      `src="${TWITCH}"`,
    );
  });

  it('renders on the server, so the art store has a server snapshot', () => {
    expect(() => render()).not.toThrow();
  });
});

// Every surface that draws a Twitch badge goes through SharpBadgeImg at its drawn
// size and keeps the attributes its <img> had.
describe('call sites', () => {
  const source = (path: string) =>
    readFileSync(resolve(__dirname, '../..', path), 'utf8').replace(/\s+/g, ' ');

  const sites: { file: string; tag: string; old: string }[] = [
    {
      file: 'components/BadgesOverlay.tsx',
      tag: '<SharpBadgeImg src={badge.image_url_4x} art={standing?.art?.[`${badge.set_id}/${badge.id}`]} cssPx={64} alt={badge.title} className="w-16 h-16 object-contain" loading="lazy" />',
      old: '<img src={badge.image_url_4x}',
    },
    {
      file: 'components/BadgeDetailOverlay.tsx',
      tag: '<SharpBadgeImg src={badge.image_url_4x} art={artMap?.[`${setId}/${badge.id}`]} cssPx={72} alt={badge.title} className="w-18 h-18 object-contain" />',
      old: '<img src={badge.image_url_4x}',
    },
    {
      file: 'components/badge/MissingNowStrip.tsx',
      tag: '<SharpBadgeImg src={badge.image_url} art={art} cssPx={40} alt="" className="w-10 h-10 object-contain drop-shadow-[0_0_10px_color-mix(in_srgb,var(--color-success)_35%,transparent)]" loading="lazy" draggable={false} />',
      old: '<img src={badge.image_url} ',
    },
    {
      file: 'mobile/rewards/MobileMissingNow.tsx',
      tag: '<SharpBadgeImg src={row.image} art={row.art} cssPx={36} alt="" className="w-9 h-9 object-contain shrink-0" draggable={false} />',
      old: '<img src={row.image}',
    },
    {
      file: 'mobile/screens/RewardsScreen.tsx',
      tag: "<SharpBadgeImg src={tile.image} art={tile.art} cssPx={44} alt=\"\" loading=\"lazy\" decoding=\"async\" className={`w-11 h-11 object-contain ${lit ? '' : 'opacity-45 grayscale'}`} draggable={false} />",
      old: '<img src={tile.image}',
    },
    {
      file: 'mobile/screens/RewardsScreen.tsx',
      tag: '<SharpBadgeImg src={badgeDetail.image} art={badgeDetail.art} cssPx={80} alt="" className="w-20 h-20 object-contain mb-2.5" draggable={false} />',
      old: '<img src={badgeDetail.image}',
    },
  ];

  for (const site of sites) {
    it(`${site.file}: ${site.old.slice(10)}`, () => {
      const text = source(site.file);
      expect(text).toContain(site.tag);
      expect(text).not.toContain(site.old);
    });
  }

  it('the desktop strip hands each card its art from the standing', () => {
    expect(source('components/badge/MissingNowStrip.tsx')).toContain('art={standing?.art?.[badge.key]}');
  });
});
