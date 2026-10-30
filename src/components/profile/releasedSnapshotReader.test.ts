// The server writes `user_profile_snapshot` rows that already-released apps
// (v8.8.18 and earlier) read with their own code. This reads the golden row the
// way that code does, so a row the server builds can never break an old card:
// `badge/undefined` images, a throwing `provider.toUpperCase()`, or a path on
// someone's disk. The same row is checked against the site builder (in the site
// repo) and against the Rust reader (`card_from`).
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

interface GoldenRow {
  v: number;
  identity: { login: string; displayName: string; avatar: string };
  profileTheme: string;
  hiddenSections: string[];
  memberNumber: number | null;
  cosmeticSlug: string | null;
  namePaint: Record<string, unknown> | null;
  seventvBadge: { id: string; name: string; tooltip?: string } | null;
  wornBadges: {
    twitch: { src: string; title: string } | null;
    thirdParty: Array<{ key?: string; provider: string; title: string; src: string }>;
    bttvPro: { src: string; title: string } | null;
  };
  counts: { paints: number; badges: number; sn: number };
}

const text = readFileSync(resolve(__dirname, '__fixtures__', 'profile-snapshot.golden.json'), 'utf8');
const golden = JSON.parse(text) as GoldenRow;

// v8.8.18 `getBadgeImageUrls` and the helper it uses, copied verbatim from
// src/services/seventvService.ts at that tag. Copied rather than imported: the
// current function behaves the same for a badge with no `images` (the only shape
// the row carries), but a released app is frozen, and this test must keep
// reading the row the way it does.
interface BadgeImageV4 {
  url: string;
  mime?: string;
  scale?: number;
  frameCount?: number;
}
interface BadgeV4 {
  id: string;
  name: string;
  description?: string;
  selected?: boolean;
  localUrl?: string;
  images?: BadgeImageV4[];
}
const pickBadgeImage = (images: BadgeImageV4[] | undefined, scale: number): string | undefined => {
  if (!images?.length) return undefined;
  const rank = (img: BadgeImageV4): number => {
    let s = Math.abs((img.scale ?? 1) - scale) * 10;
    if (img.url.includes('_static')) s += 3;
    const m = img.mime ?? '';
    s += m.includes('webp') ? 0 : m.includes('avif') ? 1 : m.includes('png') ? 2 : 4;
    return s;
  };
  return [...images].sort((a, b) => rank(a) - rank(b))[0]?.url;
};
const getBadgeImageUrls = (badge: BadgeV4): { url1x: string; url2x: string; url3x: string; url4x: string } => {
  if (badge.localUrl) {
    return { url1x: badge.localUrl, url2x: badge.localUrl, url3x: badge.localUrl, url4x: badge.localUrl };
  }
  const legacy = `https://cdn.7tv.app/badge/${badge.id}`;
  return {
    url1x: pickBadgeImage(badge.images, 1) ?? `${legacy}/1x.webp`,
    url2x: pickBadgeImage(badge.images, 2) ?? `${legacy}/2x.webp`,
    url3x: pickBadgeImage(badge.images, 3) ?? `${legacy}/3x.webp`,
    url4x: pickBadgeImage(badge.images, 4) ?? `${legacy}/4x.webp`,
  };
};

// The keys the app's `computePaintStyleUncached` (src/services/paintStyle.ts)
// can emit, and nothing else.
const PAINT_KEYS = new Set([
  'backgroundColor',
  'WebkitBackgroundClip',
  'backgroundClip',
  'backgroundSize',
  'backgroundPosition',
  'backgroundRepeat',
  'color',
  'backgroundImage',
  'filter',
  'opacity',
]);

describe('the released app reads a server-written row', () => {
  it('is a version 1 row', () => {
    expect(golden.v === 1).toBe(true);
  });

  it('draws the 7TV badge from the CDN by id and names it', () => {
    const badge = golden.seventvBadge as BadgeV4 & { tooltip?: string };
    expect(getBadgeImageUrls(badge).url4x).toBe('https://cdn.7tv.app/badge/01KYQHQ7FVRR3X4S0F2KG74207/4x.webp');
    expect(badge.tooltip || badge.name).toBe('XDX');
  });

  it('labels every third-party badge with a string provider', () => {
    expect(golden.wornBadges.thirdParty.length).toBeGreaterThan(0);
    for (const b of golden.wornBadges.thirdParty) {
      expect(typeof b.provider).toBe('string');
      expect(`${b.title} (${b.provider.toUpperCase()})`).toBe('FFZ Supporter (FFZ)');
      expect(b.key || b.title).toBe('ffz:ffz-3');
    }
  });

  it('reads the fields the card reads, none missing', () => {
    expect(golden.identity.login).toBe('golden_member');
    expect(golden.profileTheme).toBe('dispersion');
    expect(golden.hiddenSections).toEqual(['roast']);
    expect(golden.counts).toEqual({ paints: 12, badges: 7, sn: 3 });
    expect(golden.wornBadges.twitch?.src.startsWith('https://')).toBe(true);
    expect(golden.wornBadges.bttvPro?.src.startsWith('https://')).toBe(true);
  });

  it('draws the name paint from keys the card knows, with images from 7TV only', () => {
    const paint = golden.namePaint as Record<string, unknown>;
    expect(Object.keys(paint).length).toBeGreaterThan(0);
    for (const [key, value] of Object.entries(paint)) {
      expect(PAINT_KEYS.has(key), key).toBe(true);
      for (const m of String(value).matchAll(/url\(\s*["']?([^)"']*)/gi)) {
        expect(new URL(m[1]).host).toBe('cdn.7tv.app');
      }
    }
    expect(String(paint.backgroundImage)).toContain('url(');
  });

  it('carries no path to anyone\'s disk', () => {
    expect(text).not.toMatch(/localUrl|asset\.localhost|asset:\/\/|127\.0\.0\.1|[A-Za-z]:\\\\|%3A%5C/i);
  });
});
