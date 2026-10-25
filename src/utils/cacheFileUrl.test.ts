import { describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';

vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: (p: string, protocol = 'asset') => `http://${protocol}.localhost/${encodeURIComponent(p)}`,
  invoke: vi.fn(),
}));

import { __setCacheFileBase, cacheFileUrl, isDiskCopyUrl, withoutLoopbackUrls } from './cacheFileUrl';

describe('cacheFileUrl', () => {
  it('uses the asset URL until the server is known', () => {
    __setCacheFileBase(null);
    expect(cacheFileUrl('C:\\x\\universal\\emotes\\a@1x.webp')).toBe(
      `http://asset.localhost/${encodeURIComponent('C:\\x\\universal\\emotes\\a@1x.webp')}`,
    );
  });

  it('maps a cache file to the loopback route by folder and file', () => {
    __setCacheFileBase('http://127.0.0.1:5000/cache/t/');
    expect(cacheFileUrl('C:\\Users\\John (Work)\\cache\\universal\\emotes\\a@1x.webp')).toBe(
      'http://127.0.0.1:5000/cache/t/emotes/a%401x.webp',
    );
    expect(cacheFileUrl('/home/x/universal/badges/twitch-1.bin')).toBe('http://127.0.0.1:5000/cache/t/badges/twitch-1.bin');
    expect(cacheFileUrl('C:\\elsewhere\\upload.png').startsWith('http://asset.localhost/')).toBe(true);
  });

  it('knows both disk forms and nothing else', () => {
    __setCacheFileBase('http://127.0.0.1:5000/cache/t/');
    expect(isDiskCopyUrl('http://127.0.0.1:5000/cache/t/emotes/a.webp')).toBe(true);
    expect(isDiskCopyUrl('http://asset.localhost/x')).toBe(true);
    expect(isDiskCopyUrl('asset://localhost/x')).toBe(true);
    expect(isDiskCopyUrl('https://cdn.7tv.app/emote/x/1x.avif')).toBe(false);
    expect(isDiskCopyUrl('http://localhost:12345/stream.m3u8')).toBe(false);
    expect(isDiskCopyUrl(undefined)).toBe(false);
  });

  it('saved JSON never carries a loopback URL', () => {
    const saved = JSON.stringify(
      {
        badges: [{ id: 'a', localUrl: 'http://127.0.0.1:5000/cache/t/cosmetics/a.webp' }],
        paint: { images: [{ url: 'https://cdn.7tv.app/x', localUrl: 'http://127.0.0.1:61234/cache/zz/cosmetics/p.webp' }] },
        old: { localUrl: 'http://asset.localhost/C%3A%5Cx' },
      },
      withoutLoopbackUrls,
    );
    expect(saved).not.toContain('127.0.0.1');
    expect(saved).toContain('asset.localhost');
    expect(saved).toContain('https://cdn.7tv.app/x');
  });

  it('no cache service converts a cache path with convertFileSrc directly', () => {
    for (const f of [
      'src/services/emoteService.ts',
      'src/services/badgeImageCacheService.ts',
      'src/services/seventvService.ts',
      'src/services/emoteMatch.ts',
      'src/services/streamnookArt.ts',
    ]) {
      expect(readFileSync(f, 'utf8'), f).not.toMatch(/convertFileSrc\(/);
    }
  });
});
