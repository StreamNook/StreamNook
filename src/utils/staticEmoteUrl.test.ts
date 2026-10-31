import { describe, expect, it } from 'vitest';
import { hasNoStillFrame, markNoStillFrame, sevenTvSrcSet, staticEmoteUrl, static7tvSrcSet } from './staticEmoteUrl';

describe('staticEmoteUrl', () => {
  it('rewrites each provider to its static variant', () => {
    expect(staticEmoteUrl('https://cdn.7tv.app/emote/abc/1x.avif')).toBe('https://cdn.7tv.app/emote/abc/1x_static.avif');
    expect(staticEmoteUrl('https://cdn.7tv.app/emote/abc/2x.webp')).toBe('https://cdn.7tv.app/emote/abc/2x_static.webp');
    // BTTV serves first frames as WebP only: `static/1x` is a 404.
    expect(staticEmoteUrl('https://cdn.betterttv.net/emote/5f1b0186cf6d2144653d2970/1x')).toBe(
      'https://cdn.betterttv.net/emote/5f1b0186cf6d2144653d2970/static/1x.webp',
    );
    expect(staticEmoteUrl('https://cdn.betterttv.net/emote/5f1b0186cf6d2144653d2970/2x.webp')).toBe(
      'https://cdn.betterttv.net/emote/5f1b0186cf6d2144653d2970/static/2x.webp',
    );
    expect(staticEmoteUrl('https://cdn.frankerfacez.com/emote/123/animated/1')).toBe('https://cdn.frankerfacez.com/emote/123/1');
    expect(staticEmoteUrl('https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/2.0')).toBe(
      'https://static-cdn.jtvnw.net/emoticons/v2/25/static/dark/2.0',
    );
  });

  it('is idempotent and leaves unknown or local urls alone', () => {
    const s = 'https://cdn.7tv.app/emote/abc/1x_static.avif';
    expect(staticEmoteUrl(s)).toBe(s);
    expect(staticEmoteUrl('https://cdn.betterttv.net/emote/x/static/1x.webp')).toBe('https://cdn.betterttv.net/emote/x/static/1x.webp');
    expect(staticEmoteUrl('asset://localhost/C:/cache/x.webp')).toBe('asset://localhost/C:/cache/x.webp');
    expect(staticEmoteUrl('https://example.com/e.gif')).toBe('https://example.com/e.gif');
  });

  it('remembers emotes that have no first frame, per provider', () => {
    expect(hasNoStillFrame('7tv', 'kekw')).toBe(false);
    markNoStillFrame('7tv', 'kekw');
    expect(hasNoStillFrame('7tv', 'kekw')).toBe(true);
    expect(hasNoStillFrame('bttv', 'kekw')).toBe(false);
    markNoStillFrame(undefined, 'x');
    expect(hasNoStillFrame(undefined, 'x')).toBe(false);
  });

  it('builds a static 7tv srcSet', () => {
    expect(static7tvSrcSet('id')).toContain('https://cdn.7tv.app/emote/id/2x_static.avif 2x');
  });

  it('builds the WebP srcSet a WebView without AVIF falls back to', () => {
    expect(sevenTvSrcSet('id', 'webp')).toBe(
      'https://cdn.7tv.app/emote/id/1x.webp 1x, https://cdn.7tv.app/emote/id/2x.webp 2x, https://cdn.7tv.app/emote/id/3x.webp 3x, https://cdn.7tv.app/emote/id/4x.webp 4x',
    );
    expect(sevenTvSrcSet('id', 'webp', true)).toContain('https://cdn.7tv.app/emote/id/1x_static.webp 1x');
    expect(sevenTvSrcSet('id', 'avif')).toContain('https://cdn.7tv.app/emote/id/4x.avif 4x');
  });
});
