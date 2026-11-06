// @vitest-environment node
import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { createElement } from 'react';
import { GlassHeart, SubscribeBadge, favoriteLabel } from './menuIcons';

describe('favorite glyph and words', () => {
  it('says what a press will do', () => {
    expect(favoriteLabel(false)).toBe('Favorite');
    expect(favoriteLabel(true)).toBe('Unfavorite');
  });

  it("is desktop's glass heart when favorited, a quiet outline when not, never a star", () => {
    const on = renderToStaticMarkup(createElement(GlassHeart, { on: true, size: 22 }));
    const off = renderToStaticMarkup(createElement(GlassHeart, { on: false, size: 22 }));
    expect(on).toContain('lucide-heart');
    expect(on).toContain('url(#sn-m-glass-heart-fill)');
    expect(on).toContain('url(#sn-m-glass-heart-stroke)');
    expect(off).toContain('lucide-heart');
    expect(off).toContain('text-textSecondary');
    expect(off).not.toContain('glass-heart');
    expect(on + off).not.toContain('star');
  });
});

describe('subscribe badge slot', () => {
  it('draws the badge art when Rust has answered', () => {
    const html = renderToStaticMarkup(createElement(SubscribeBadge, { url: 'https://badge.test/1.png', size: 22 }));
    expect(html).toContain('<img');
    expect(html).toContain('https://badge.test/1.png');
  });

  it('keeps an empty slot of the same size while the art loads, never a stand-in glyph', () => {
    const html = renderToStaticMarkup(createElement(SubscribeBadge, { url: null, size: 22 }));
    expect(html).not.toContain('<svg');
    expect(html).not.toContain('<img');
    expect(html).toContain('width:22px');
    expect(html).toContain('height:22px');
  });
});
