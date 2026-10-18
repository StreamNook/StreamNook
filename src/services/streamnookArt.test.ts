// @vitest-environment node
//
// StreamNook art on cdn.streamnook.app is asked of Rust's disk cache; once a copy
// lands the page loads the file instead of the CDN, so art the webview could not
// fetch still shows. Any other URL passes through untouched.
import { beforeEach, describe, expect, it, vi } from 'vitest';

const hoisted = vi.hoisted(() => ({
  sink: null as null | { arrived: (files: Record<string, string>) => void; cleared: () => void },
  requests: [] as Array<[string, string, string]>,
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(() => Promise.resolve({})),
  convertFileSrc: (path: string) => `asset://localhost/${path}`,
}));

vi.mock('./assetCacheQueue', () => ({
  onAssetsCached: (_kind: string, sink: typeof hoisted.sink) => {
    hoisted.sink = sink;
  },
  requestAssetCaching: (kind: string, id: string, url: string) => {
    hoisted.requests.push([kind, id, url]);
  },
}));

import {
  getStreamnookArtVersion,
  requestStreamnookArt,
  streamnookArtSrc,
  streamnookArtUrlsIn,
  subscribeStreamnookArt,
  withStreamnookArtCopies,
} from './streamnookArt';

const KINDRED = 'https://cdn.streamnook.app/badges/kindred.webp?v=2';
const MIDNIGHT = 'https://cdn.streamnook.app/atmospheres/midnight.webp';

describe('streamnookArtSrc', () => {
  beforeEach(() => {
    hoisted.requests.length = 0;
    hoisted.sink?.cleared();
  });

  it('leaves bundled and third-party art alone', () => {
    expect(streamnookArtSrc('/assets/streamnook-badge-gold-128.webp')).toBe('/assets/streamnook-badge-gold-128.webp');
    expect(streamnookArtSrc('https://static-cdn.jtvnw.net/badges/v1/x/3')).toBe('https://static-cdn.jtvnw.net/badges/v1/x/3');
    expect(hoisted.requests).toEqual([]);
  });

  it('serves the CDN until a copy lands, and asks Rust for one keyed by URL', () => {
    expect(streamnookArtSrc(KINDRED)).toBe(KINDRED);
    expect(hoisted.requests).toEqual([['streamnook', KINDRED, KINDRED]]);
  });

  it('switches to the disk copy when it lands and tells subscribers', () => {
    const heard = vi.fn();
    const unsubscribe = subscribeStreamnookArt(heard);
    const before = getStreamnookArtVersion();

    hoisted.sink!.arrived({ [KINDRED]: 'C:/cache/streamnook-art/kindred-1a2b3c4d.webp' });

    expect(heard).toHaveBeenCalled();
    expect(getStreamnookArtVersion()).toBeGreaterThan(before);
    expect(streamnookArtSrc(KINDRED)).toBe('asset://localhost/C:/cache/streamnook-art/kindred-1a2b3c4d.webp');
    unsubscribe();
  });

  it('falls back to the CDN after the cache is cleared', () => {
    hoisted.sink!.arrived({ [KINDRED]: 'C:/cache/streamnook-art/kindred-1a2b3c4d.webp' });
    hoisted.sink!.cleared();
    expect(streamnookArtSrc(KINDRED)).toBe(KINDRED);
  });
});

// Atmospheres name their art inside CSS (`url(...) center / cover`) as well as
// on its own, and the catalog is mapped without fetching anything.
describe('atmosphere art in CSS', () => {
  const swatch = `url(${MIDNIGHT}) center / cover`;

  beforeEach(() => {
    hoisted.requests.length = 0;
    hoisted.sink?.cleared();
  });

  it('finds the art a value names, and nothing in plain CSS', () => {
    expect(streamnookArtUrlsIn(swatch)).toEqual([MIDNIGHT]);
    expect(streamnookArtUrlsIn(`url("${MIDNIGHT}"), url('${KINDRED}')`)).toEqual([MIDNIGHT, KINDRED]);
    expect(streamnookArtUrlsIn('linear-gradient(135deg, #10b981, #8b5cf6)')).toEqual([]);
    expect(streamnookArtUrlsIn(undefined)).toEqual([]);
  });

  it('maps to disk copies without asking for anything', () => {
    expect(withStreamnookArtCopies(swatch)).toBe(swatch);
    expect(withStreamnookArtCopies(undefined)).toBeUndefined();
    expect(hoisted.requests).toEqual([]);

    hoisted.sink!.arrived({ [MIDNIGHT]: 'C:/cache/streamnook-art/midnight-9f8e7d6c.webp' });
    expect(withStreamnookArtCopies(swatch)).toBe(
      'url(asset://localhost/C:/cache/streamnook-art/midnight-9f8e7d6c.webp) center / cover',
    );
    expect(withStreamnookArtCopies(MIDNIGHT)).toBe('asset://localhost/C:/cache/streamnook-art/midnight-9f8e7d6c.webp');
  });

  it('keeps a copy in a folder with brackets inside its url(...)', () => {
    hoisted.sink!.arrived({ [MIDNIGHT]: "C:/Users/O'Neil (Work)/cache/midnight.webp" });
    expect(withStreamnookArtCopies(swatch)).toBe(
      'url(asset://localhost/C:/Users/O%27Neil %28Work%29/cache/midnight.webp) center / cover',
    );
  });

  it('asks for a copy only while there is none', () => {
    requestStreamnookArt(MIDNIGHT);
    requestStreamnookArt('https://example.com/x.webp');
    expect(hoisted.requests).toEqual([['streamnook', MIDNIGHT, MIDNIGHT]]);

    hoisted.sink!.arrived({ [MIDNIGHT]: 'C:/cache/streamnook-art/midnight-9f8e7d6c.webp' });
    requestStreamnookArt(MIDNIGHT);
    expect(hoisted.requests).toHaveLength(1);
  });
});
