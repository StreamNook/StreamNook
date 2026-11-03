// @vitest-environment node
//
// A row's badges past the limit fold into one "+N" pill that shows the next
// two hidden badges' art. The static markup is the closed state: the strip
// itself is portaled only while open.
import { describe, expect, it, vi } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

// The browser globals the app's stores read at import (the same stand-ins
// ChatBadgeIcons.test.tsx uses).
vi.hoisted(() => {
  const store = new Map<string, string>();
  const storage = {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, String(v)),
    removeItem: (k: string) => void store.delete(k),
    clear: () => store.clear(),
    key: () => null,
    length: 0,
  };
  const noop = () => {};
  const el = () => ({ style: {}, dataset: {} as Record<string, string>, setAttribute: noop, appendChild: noop, classList: { add: noop, remove: noop, toggle: noop }, addEventListener: noop, removeEventListener: noop });
  const g = globalThis as Record<string, unknown>;
  g.localStorage = storage;
  g.sessionStorage = storage;
  g.document = { documentElement: el(), body: el(), head: el(), createElement: el, addEventListener: noop, removeEventListener: noop, querySelector: () => null, querySelectorAll: () => [], visibilityState: 'visible', fonts: { ready: Promise.resolve() } };
  g.window = Object.assign(globalThis, {
    location: { hash: '', href: 'http://localhost/', origin: 'http://localhost', search: '' },
    addEventListener: noop,
    removeEventListener: noop,
    matchMedia: () => ({ matches: false, addEventListener: noop, removeEventListener: noop, addListener: noop, removeListener: noop }),
    requestAnimationFrame: (cb: () => void) => setTimeout(cb, 0),
    cancelAnimationFrame: noop,
    __TAURI_INTERNALS__: { invoke: () => Promise.resolve(null), transformCallback: () => 0, metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } } },
  });
  if (!g.navigator) g.navigator = { userAgent: 'node', platform: 'Win32', language: 'en-US' };
});

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(() => Promise.resolve(null)),
  convertFileSrc: (p: string) => p,
  Channel: class {},
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})), emit: vi.fn() }));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ label: 'main' }) }));
vi.mock('@tauri-apps/api/webviewWindow', () => ({ WebviewWindow: class {}, getCurrentWebviewWindow: () => ({ label: 'main' }) }));
// The member badge reads the member registry; here it is one image like the others.
vi.mock('../StreamNookBadge', async () => {
  const { createElement } = await import('react');
  return {
    StreamNookBadge: ({ className }: { className?: string }) => createElement('img', { src: '/sn.webp', alt: '', className }),
  };
});
vi.mock('../../utils/openBadgesInMain', () => ({
  openBadgeDetailInMain: vi.fn(),
  openBadgesWithBadgeInMain: vi.fn(),
  openBadgesOnStreamNookInMain: vi.fn(),
  openBadgesWithTargetInMain: vi.fn(),
}));
vi.mock('../../utils/lowPower', () => ({ useLowPower: () => false, useStillChat: () => false }));
// The failed-art read, settable per case: the row's failed urls, each ending "\n".
const failedArt = vi.hoisted(() => ({ value: '' }));
vi.mock('../../utils/badgeArtFailures', () => ({
  useFailedBadgeArt: () => failedArt.value,
  markBadgeArtFailed: () => {},
}));

import { ChatBadgeIcons, type ResolvedTwitchBadge } from './ChatBadgeIcons';
import type { ThirdPartyBadge } from '../../services/badgeService';

const twitch = (key: string, art: string, provider?: string): ResolvedTwitchBadge => ({
  key,
  info: { title: key, localUrl: art, image_url_4x: art, provider },
});
const addon = (id: string, provider: string): ThirdPartyBadge => ({
  id,
  title: id,
  imageUrl: `/${id}.png`,
  image1x: `/${id}.png`,
  image2x: `/${id}.png`,
  image4x: `/${id}.png`,
  provider,
});

const MOD = twitch('moderator/1', '/mod.png');
const SUB = twitch('subscriber/12', '/sub.png');
const BITS = twitch('bits/1000', '/bits.png');
const SEVENTV = { id: '01HSUB', name: '7TV Subscriber', description: '7TV Subscriber - 6 Months' };
const FFZ = addon('ffz-3', 'ffz');
const CHATTERINO = addon('chatterino-1', 'chatterino');

function html(props: Partial<Parameters<typeof ChatBadgeIcons>[0]>): string {
  return renderToStaticMarkup(
    <ChatBadgeIcons twitchBadges={[]} seventvBadge={null} thirdPartyBadges={[]} streamNookMemberId={null} {...props} />,
  );
}

/** The `<img>` tags of the row's drawn badges (not the pill's peek thumbnails). */
const badgeImages = (markup: string): string[] =>
  (markup.match(/<img[^>]*>/g) ?? []).filter((t) => t.includes('class="sn-chat-badge'));
const peekImages = (markup: string): string[] =>
  (markup.match(/<img[^>]*>/g) ?? []).filter((t) => t.includes('sn-badge-fold-peek'));

describe('ChatBadgeIcons folding', () => {
  // Three Twitch badges and four add-ons (7TV, FFZ, Chatterino, the member badge).
  const seven = {
    twitchBadges: [MOD, SUB, BITS],
    seventvBadge: SEVENTV,
    thirdPartyBadges: [FFZ, CHATTERINO],
    streamNookMemberId: '123',
  };

  it("always shows Twitch's own badges and keeps the member badge first among the add-ons", () => {
    const markup = html({ ...seven, foldLimit: 2 });
    const shown = badgeImages(markup);
    expect(shown).toHaveLength(5);
    expect(shown[0]).toContain('src="/mod.png"');
    expect(shown[1]).toContain('src="/sub.png"');
    expect(shown[2]).toContain('src="/bits.png"');
    expect(shown[3]).toContain('01HSUB');
    expect(shown[4]).toContain('src="/sn.webp"');
    expect(markup).toContain('aria-label="2 more badges"');
    const peeks = peekImages(markup);
    expect(peeks).toHaveLength(2);
    expect(peeks[0]).toContain('src="/ffz-3.png"');
    expect(peeks[1]).toContain('src="/chatterino-1.png"');
    expect(markup).toMatch(/\+2<\/span><\/button>/);
    expect(markup).not.toContain('role="group"');
  });

  it('gives the pill a rarity tier from every add-on, whatever the limit', () => {
    expect(html({ ...seven, foldLimit: 2 })).toContain('data-tier="3"');
    const higher = html({ ...seven, foldLimit: 3 });
    expect(higher).toContain('aria-label="1 more badge"');
    expect(higher).toContain('data-tier="3"');
  });

  it('shows every badge at limit 0', () => {
    const markup = html({ ...seven, foldLimit: 0 });
    expect(badgeImages(markup)).toHaveLength(7);
    expect(markup).not.toContain('sn-badge-fold-chip');
  });

  it('folds a single add-on past the limit too: the setting means what it says', () => {
    const markup = html({ twitchBadges: [MOD], seventvBadge: SEVENTV, thirdPartyBadges: [FFZ], streamNookMemberId: '123', foldLimit: 2 });
    expect(badgeImages(markup)).toHaveLength(3);
    expect(markup).toContain('aria-label="1 more badge"');
    expect(peekImages(markup)[0]).toContain('/ffz-3.png');
    expect(markup).toContain('data-tier="2"');
  });

  it("keeps every Kick badge, the subscriber badge before the moderator's too", () => {
    const markup = html({
      twitchBadges: [twitch('subscriber/1', '/kick-sub.svg', 'kick'), twitch('moderator/1', '/kick-mod.svg', 'kick')],
      seventvBadge: SEVENTV,
      thirdPartyBadges: [FFZ, CHATTERINO, addon('chatty-1', 'chatty')],
      streamNookMemberId: '123',
      foldLimit: 2,
    });
    const shown = badgeImages(markup).join('');
    expect(shown).toContain('/kick-mod.svg');
    expect(shown).toContain('/kick-sub.svg');
    expect(markup).toContain('aria-label="3 more badges"');
  });

  it('drops a badge whose art failed before folding: the count, peeks and tier skip it', () => {
    failedArt.value = '/ffz-3.png\n';
    try {
      const markup = html({ ...seven, foldLimit: 2 });
      expect(markup).toContain('aria-label="1 more badge"');
      expect(peekImages(markup).join('')).not.toContain('/ffz-3.png');
      expect(markup).not.toContain('src="/ffz-3.png"');
      expect(markup).toContain('data-tier="2"');
    } finally {
      failedArt.value = '';
    }
  });

  it('a failure that leaves the add-ons within the limit folds nothing', () => {
    failedArt.value = '/chatterino-1.png\n';
    try {
      const markup = html({ twitchBadges: [MOD], seventvBadge: SEVENTV, thirdPartyBadges: [CHATTERINO], streamNookMemberId: '123', foldLimit: 2 });
      expect(badgeImages(markup)).toHaveLength(3);
      expect(markup).not.toContain('sn-badge-fold-chip');
    } finally {
      failedArt.value = '';
    }
  });

  it('draws the pill as a plain face inside another control', () => {
    const markup = html({ ...seven, foldLimit: 2, interactive: false });
    expect(markup).toContain('<span class="sn-badge-fold-chip" data-tier="3" aria-label="2 more badges">');
    expect(markup).not.toContain('<button');
  });
});
