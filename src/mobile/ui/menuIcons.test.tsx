// @vitest-environment node
import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { createElement } from 'react';
import { FavoriteStar, SubscribeBadge, favoriteLabel } from './menuIcons';

describe('favorite glyph and words', () => {
  it('says what a press will do', () => {
    expect(favoriteLabel(false)).toBe('Favorite');
    expect(favoriteLabel(true)).toBe('Unfavorite');
  });

  it('is a gold star when favorited, a quiet outline when not', () => {
    const on = renderToStaticMarkup(createElement(FavoriteStar, { on: true, size: 22 }));
    const off = renderToStaticMarkup(createElement(FavoriteStar, { on: false, size: 22 }));
    expect(on).toContain('text-yellow-400');
    expect(off).toContain('text-textSecondary');
    expect(off).not.toContain('text-yellow-400');
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
