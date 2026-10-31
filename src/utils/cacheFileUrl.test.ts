import { describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';

vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: (p: string, protocol = 'asset') => `http://${protocol}.localhost/${encodeURIComponent(p)}`,
  invoke: vi.fn(),
}));

import {
  __setCacheFileBase,
  awaitCacheFileServer,
  cacheFileUrl,
  isDiskCopyUrl,
  probeLost,
  usesLoopback,
  withoutLoopbackUrls,
} from './cacheFileUrl';
import { invoke } from '@tauri-apps/api/core';

const ANDROID_UA = 'Mozilla/5.0 (Linux; Android 15; CPH2841) AppleWebKit/537.36 Chrome/141 Mobile';

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

  it('uses loopback on Windows and Android only', () => {
    expect(usesLoopback('Mozilla/5.0 (Windows NT 10.0; Win64; x64) Edg/141')).toBe(true);
    expect(usesLoopback(ANDROID_UA)).toBe(true);
    expect(usesLoopback('Mozilla/5.0 (Macintosh; Intel Mac OS X 15_0) AppleWebKit/605.1.15')).toBe(false);
    expect(usesLoopback('Mozilla/5.0 (X11; Linux x86_64) Chrome/141')).toBe(false);
    expect(usesLoopback('Mozilla/5.0 (iPhone; CPU iPhone OS 26_0 like Mac OS X)')).toBe(false);
  });

  it('an Android page draws cached files from the loopback server', async () => {
    vi.resetModules();
    vi.stubGlobal('navigator', { userAgent: ANDROID_UA });
    try {
      const core = await import('@tauri-apps/api/core');
      vi.mocked(core.invoke).mockResolvedValue({ port: 5002, token: 'a' });
      const fresh = await import('./cacheFileUrl');
      await fresh.ensureCacheFileServer();
      expect(fresh.cacheFileUrl('/d/universal/emotes/a@1x.webp')).toBe('http://127.0.0.1:5002/cache/a/emotes/a%401x.webp');
    } finally {
      vi.unstubAllGlobals();
      vi.resetModules();
    }
  });

  it('only Android pages wait for the server before painting', async () => {
    for (const [agent, waits] of [
      [ANDROID_UA, true],
      ['Mozilla/5.0 (Windows NT 10.0; Win64; x64) Edg/141', false],
      ['Mozilla/5.0 (Macintosh; Intel Mac OS X 15_0) AppleWebKit/605.1.15', false],
      ['Mozilla/5.0 (X11; Linux x86_64) Chrome/141', false],
    ] as const) {
      vi.resetModules();
      vi.stubGlobal('navigator', { userAgent: agent });
      const fresh = await import('./cacheFileUrl');
      expect(fresh.waitsForCacheFileServer(), agent).toBe(waits);
      vi.unstubAllGlobals();
    }
    vi.resetModules();
  });

  it('Android asks again until the server has bound', async () => {
    __setCacheFileBase(null);
    const ask = vi.mocked(invoke);
    ask.mockReset();
    ask.mockResolvedValueOnce(null).mockResolvedValueOnce(null).mockResolvedValue({ port: 5001, token: 'k' });
    await awaitCacheFileServer({ android: true, capMs: 2000, stepMs: 1 });
    expect(ask).toHaveBeenCalledTimes(3);
    expect(cacheFileUrl('/d/universal/emotes/a@1x.webp')).toBe('http://127.0.0.1:5001/cache/k/emotes/a%401x.webp');
  });

  it('keeps asking until the cap, then leaves the asset protocol', async () => {
    __setCacheFileBase(null);
    const ask = vi.mocked(invoke);
    ask.mockReset();
    ask.mockResolvedValue(null);
    await awaitCacheFileServer({ android: true, capMs: 30, stepMs: 5 });
    expect(ask.mock.calls.length).toBeGreaterThan(1);
    expect(cacheFileUrl('/d/universal/emotes/a@1x.webp').startsWith('http://asset.localhost/')).toBe(true);
  });

  it('a hung ask still paints at the cap', async () => {
    __setCacheFileBase(null);
    const ask = vi.mocked(invoke);
    ask.mockReset();
    ask.mockReturnValue(new Promise(() => {}));
    const t0 = Date.now();
    await awaitCacheFileServer({ android: true, capMs: 20, stepMs: 5 });
    expect(Date.now() - t0).toBeLessThan(1000);
    expect(cacheFileUrl('/d/universal/emotes/a@1x.webp').startsWith('http://asset.localhost/')).toBe(true);
    __setCacheFileBase(null);
  });

  it('stops at once when the server gave up', async () => {
    __setCacheFileBase(null);
    const ask = vi.mocked(invoke);
    ask.mockReset();
    ask.mockRejectedValue('unavailable');
    const t0 = Date.now();
    await awaitCacheFileServer({ android: true, capMs: 2000, stepMs: 5 });
    expect(Date.now() - t0).toBeLessThan(500);
    expect(ask).toHaveBeenCalledTimes(1);
    __setCacheFileBase(null);
  });

  it('a window that is not Android does not wait', async () => {
    const ask = vi.mocked(invoke);
    ask.mockReset();
    await awaitCacheFileServer({ android: false });
    expect(ask).not.toHaveBeenCalled();
  });

  it('the server is lost only after every ping fails', async () => {
    const answers = (codes: (number | Error)[]) => {
      let i = 0;
      return async () => {
        const c = codes[Math.min(i++, codes.length - 1)];
        if (c instanceof Error) throw c;
        return { status: c };
      };
    };
    expect(await probeLost('http://x/', 3, 0, answers([new Error('down'), 500, 204]))).toBe(false);
    expect(await probeLost('http://x/', 3, 0, answers([new Error('down'), new Error('down'), new Error('down')]))).toBe(true);
    expect(await probeLost('http://x/', 1, 0, answers([new Error('down'), 204]))).toBe(true);
    const src = readFileSync('src/utils/cacheFileUrl.ts', 'utf8');
    expect(src).toContain('void probeLost(probe, ANDROID ? 3 : 1, 700, ping)');
  });

  it('Android windows learn the server before their first paint; others paint at once', () => {
    const main = readFileSync('src/main.tsx', 'utf8').replace(/\r\n/g, '\n');
    expect(main).toContain('const paint = () => root.render(');
    expect(main).toContain('if (waitsForCacheFileServer()) void awaitCacheFileServer().finally(paint);\nelse paint();');
    expect(main.match(/root\.render\(/g)).toHaveLength(1);
  });
});
