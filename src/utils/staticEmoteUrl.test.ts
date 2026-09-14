import { describe, expect, it } from 'vitest';
import { staticEmoteUrl, static7tvSrcSet } from './staticEmoteUrl';

describe('staticEmoteUrl', () => {
  it('rewrites each provider to its static variant', () => {
    expect(staticEmoteUrl('https://cdn.7tv.app/emote/abc/1x.avif')).toBe('https://cdn.7tv.app/emote/abc/1x_static.avif');
    expect(staticEmoteUrl('https://cdn.7tv.app/emote/abc/2x.webp')).toBe('https://cdn.7tv.app/emote/abc/2x_static.webp');
    expect(staticEmoteUrl('https://cdn.betterttv.net/emote/5f1b0186cf6d2144653d2970/1x')).toBe(
      'https://cdn.betterttv.net/emote/5f1b0186cf6d2144653d2970/static/1x',
    );
    expect(staticEmoteUrl('https://cdn.frankerfacez.com/emote/123/animated/1')).toBe('https://cdn.frankerfacez.com/emote/123/1');
    expect(staticEmoteUrl('https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/2.0')).toBe(
      'https://static-cdn.jtvnw.net/emoticons/v2/25/static/dark/2.0',
    );
  });

  it('is idempotent and leaves unknown or local urls alone', () => {
    const s = 'https://cdn.7tv.app/emote/abc/1x_static.avif';
    expect(staticEmoteUrl(s)).toBe(s);
    expect(staticEmoteUrl('https://cdn.betterttv.net/emote/x/static/1x')).toBe('https://cdn.betterttv.net/emote/x/static/1x');
    expect(staticEmoteUrl('asset://localhost/C:/cache/x.webp')).toBe('asset://localhost/C:/cache/x.webp');
    expect(staticEmoteUrl('https://example.com/e.gif')).toBe('https://example.com/e.gif');
  });

  it('builds a static 7tv srcSet', () => {
    expect(static7tvSrcSet('id')).toContain('https://cdn.7tv.app/emote/id/2x_static.avif 2x');
  });
});
