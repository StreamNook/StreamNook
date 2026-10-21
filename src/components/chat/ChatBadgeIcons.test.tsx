// @vitest-environment node
//
// Every chat badge is a button. Twitch, 7TV and StreamNook badges go straight
// to their page in the Badges overlay (Twitch WITH the version id, which the
// page keys its lookups on); third-party clients and Kick / YouTube / TikTok
// badges open the small spotlight card in this window, and a platform badge
// never reaches the Twitch page.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { isValidElement, type ReactElement, type ReactNode } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

// The browser globals the app's stores read at import (the same stand-ins
// ChatMessage.eventName.test.tsx uses).
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
// The StreamNook badge reads the member registry; its props are what matter here.
vi.mock('../StreamNookBadge', () => ({ StreamNookBadge: () => null }));
const routes = vi.hoisted(() => ({
  openBadgeDetailInMain: vi.fn(),
  openBadgesWithBadgeInMain: vi.fn(),
  openBadgesOnStreamNookInMain: vi.fn(),
  openBadgesWithTargetInMain: vi.fn(),
}));
vi.mock('../../utils/openBadgesInMain', () => routes);
// Called as a plain function below, outside a React render: its Battery Saver
// hook stands in as off.
vi.mock('../../utils/lowPower', () => ({ useLowPower: () => false }));

import { ChatBadgeIcons, type ResolvedTwitchBadge } from './ChatBadgeIcons';
import { BadgeHoverCard } from './BadgeHoverCard';
import { openBadgeInfo, useBadgeSpotlightStore } from '../../stores/badgeSpotlightStore';
import { badgeDetailsRoute, directBadgeRoute, pickBadgeArt, streamNookBadgeRef, type BadgeRef } from '../../utils/badgeSpotlight';
import type { ThirdPartyBadge } from '../../services/badgeService';

type ClickableProps = { onClick?: () => void; 'aria-label'?: string; children?: ReactNode };

/** Every element in a rendered tree (ChatBadgeIcons has no hooks, so calling
 *  it returns the tree its rows would mount). */
function elements(node: ReactNode): ReactElement[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!isValidElement(node)) return [];
  const props = node.props as { children?: ReactNode };
  return [node, ...elements(props.children)];
}

function buttons(node: ReactNode): ReactElement<ClickableProps>[] {
  return elements(node).filter((e): e is ReactElement<ClickableProps> => e.type === 'button');
}

const TWITCH_SUB: ResolvedTwitchBadge = {
  key: 'subscriber/12',
  info: {
    id: '12',
    title: '1-Year Subscriber',
    image_url_1x: 'https://static-cdn.jtvnw.net/badges/v1/sub/1',
    image_url_2x: 'https://static-cdn.jtvnw.net/badges/v1/sub/2',
    image_url_4x: 'https://static-cdn.jtvnw.net/badges/v1/sub/3',
  },
};

// What twitchChat builds for a Kick row's badge.
const KICK_MOD: ResolvedTwitchBadge = {
  key: 'moderator/1',
  info: {
    localUrl: '/kick/moderator.svg',
    image_url_1x: '/kick/moderator.svg',
    image_url_4x: '/kick/moderator.svg',
    title: 'Moderator',
    provider: 'kick',
  },
};

const FFZ_LOADOUT: ThirdPartyBadge = {
  id: 'ffz:ffz-3',
  title: 'FFZ Supporter',
  imageUrl: 'https://cdn.frankerfacez.com/badge/3/4',
  image1x: 'https://cdn.frankerfacez.com/badge/3/1',
  image2x: 'https://cdn.frankerfacez.com/badge/3/2',
  image4x: 'https://cdn.frankerfacez.com/badge/3/4',
  provider: 'ffz',
};

const BTTV_PRO: ThirdPartyBadge = {
  id: 'bttv-pro',
  title: 'BTTV Pro',
  imageUrl: 'https://cdn.betterttv.net/pro.png',
  image1x: 'https://cdn.betterttv.net/pro.png',
  image2x: 'https://cdn.betterttv.net/pro.png',
  image4x: 'https://cdn.betterttv.net/pro.png',
  provider: 'BTTV',
};

const SEVENTV = { id: '01HSUB', name: '7TV Subscriber', description: '7TV Subscriber - 6 Months' };

/** Click the one badge; the router loads on the click, so let it settle. */
async function clickDirect(props: Partial<Parameters<typeof ChatBadgeIcons>[0]>): Promise<void> {
  const tree = ChatBadgeIcons({
    twitchBadges: [],
    seventvBadge: null,
    thirdPartyBadges: [],
    streamNookMemberId: null,
    ...props,
  });
  const found = buttons(tree);
  expect(found).toHaveLength(1);
  found[0].props.onClick?.();
  await vi.dynamicImportSettled();
  // Straight to the page: the card never opens.
  expect(useBadgeSpotlightStore.getState().badge).toBeNull();
}

function clickOnly(props: Partial<Parameters<typeof ChatBadgeIcons>[0]>): BadgeRef {
  const tree = ChatBadgeIcons({
    twitchBadges: [],
    seventvBadge: null,
    thirdPartyBadges: [],
    streamNookMemberId: null,
    ...props,
  });
  const found = buttons(tree);
  expect(found).toHaveLength(1);
  found[0].props.onClick?.();
  const opened = useBadgeSpotlightStore.getState().badge;
  expect(opened).not.toBeNull();
  return opened!;
}

describe('ChatBadgeIcons click routing', () => {
  beforeEach(() => {
    useBadgeSpotlightStore.getState().close();
    for (const fn of Object.values(routes)) fn.mockClear();
  });

  it('a Twitch badge opens its Badges page directly, with the version id', async () => {
    await clickDirect({ twitchBadges: [TWITCH_SUB] });
    expect(routes.openBadgeDetailInMain).toHaveBeenCalledTimes(1);
    expect(routes.openBadgeDetailInMain.mock.calls[0][1]).toBe('subscriber');
    expect(routes.openBadgeDetailInMain.mock.calls[0][0]).toMatchObject({ id: '12', title: '1-Year Subscriber' });
  });

  it('a Twitch badge without an indexed id still gets the version from its key', async () => {
    await clickDirect({ twitchBadges: [{ key: 'premium/1', info: { title: 'Prime Gaming', image_url_4x: 'x' } }] });
    expect(routes.openBadgeDetailInMain.mock.calls[0][1]).toBe('premium');
    expect(routes.openBadgeDetailInMain.mock.calls[0][0]).toMatchObject({ id: '1' });
  });

  it('a Kick badge opens the spotlight but never the Twitch badge page', async () => {
    const ref = clickOnly({ twitchBadges: [KICK_MOD] });
    expect(ref.provider).toBe('kick');
    expect(ref.twitch).toBeUndefined();
    expect(badgeDetailsRoute(ref)).toBeNull();
    await vi.dynamicImportSettled();
    expect(routes.openBadgeDetailInMain).not.toHaveBeenCalled();
  });

  it('YouTube and TikTok badges have no badge page either', () => {
    for (const provider of ['youtube', 'tiktok']) {
      const ref = clickOnly({ twitchBadges: [{ key: 'member/1', info: { ...KICK_MOD.info!, provider } }] });
      expect(ref.provider).toBe(provider);
      expect(badgeDetailsRoute(ref)).toBeNull();
    }
  });

  it('a host override still receives Twitch-slot clicks', () => {
    const onBadgeClick = vi.fn();
    const tree = ChatBadgeIcons({
      twitchBadges: [TWITCH_SUB],
      seventvBadge: null,
      thirdPartyBadges: [],
      streamNookMemberId: null,
      onBadgeClick,
    });
    buttons(tree)[0].props.onClick?.();
    expect(onBadgeClick).toHaveBeenCalledWith('subscriber/12', TWITCH_SUB.info);
    expect(useBadgeSpotlightStore.getState().badge).toBeNull();
  });

  it('a 7TV badge opens its cosmetic in Badges directly', async () => {
    await clickDirect({ seventvBadge: SEVENTV });
    expect(routes.openBadgesWithBadgeInMain).toHaveBeenCalledWith('01HSUB');
  });

  it('a third-party badge opens the card (its only detail view), keeping its loadout id for Rust', async () => {
    const ref = clickOnly({ thirdPartyBadges: [FFZ_LOADOUT] });
    expect(ref).toMatchObject({ provider: 'ffz', id: 'ffz:ffz-3', title: 'FFZ Supporter' });
    expect(badgeDetailsRoute(ref)).toEqual({ kind: 'target', tab: 'chat-clients', query: 'FFZ Supporter' });
    expect(directBadgeRoute(ref)).toBeNull();
    await vi.dynamicImportSettled();
    expect(routes.openBadgesWithTargetInMain).not.toHaveBeenCalled();
  });

  it('a BetterTTV Pro badge (provider BTTV) routes to the BetterTTV tab', () => {
    const ref = clickOnly({ thirdPartyBadges: [BTTV_PRO] });
    expect(ref.provider).toBe('bttv');
    expect(badgeDetailsRoute(ref)).toEqual({ kind: 'target', tab: 'bttv', query: 'BTTV Pro' });
  });

  it('the StreamNook badge is made interactive and routes to the StreamNook tab', () => {
    const tree = ChatBadgeIcons({
      twitchBadges: [],
      seventvBadge: null,
      thirdPartyBadges: [],
      streamNookMemberId: '123',
    });
    const sn = elements(tree).find((e) => (e.props as { userId?: string }).userId === '123');
    expect(sn?.props).toMatchObject({ interactive: true, requirePointerMove: true });
    expect(badgeDetailsRoute({ provider: 'streamnook', id: '123', title: 'StreamNook', art: [] })).toEqual({
      kind: 'streamnook',
    });
  });

  it('a StreamNook badge opens Badges on the cosmetic it wears', async () => {
    openBadgeInfo(streamNookBadgeRef('123', 'StreamNook Carver', 'art.webp', 'carver'));
    await vi.dynamicImportSettled();
    expect(routes.openBadgesOnStreamNookInMain).toHaveBeenCalledWith('carver');
    expect(useBadgeSpotlightStore.getState().badge).toBeNull();
    openBadgeInfo(streamNookBadgeRef('123', 'StreamNook', 'mark.svg'));
    await vi.dynamicImportSettled();
    expect(routes.openBadgesOnStreamNookInMain).toHaveBeenLastCalledWith(undefined);
  });

  it('interactive={false} (inside another control) draws no buttons', () => {
    const tree = ChatBadgeIcons({
      twitchBadges: [TWITCH_SUB, KICK_MOD],
      seventvBadge: SEVENTV,
      thirdPartyBadges: [FFZ_LOADOUT],
      streamNookMemberId: null,
      interactive: false,
    });
    expect(buttons(tree)).toHaveLength(0);
  });

  it('every badge type is a real button, labelled for assistive tech', () => {
    const tree = ChatBadgeIcons({
      twitchBadges: [TWITCH_SUB, KICK_MOD],
      seventvBadge: SEVENTV,
      thirdPartyBadges: [FFZ_LOADOUT, BTTV_PRO],
      streamNookMemberId: null,
    });
    const labels = buttons(tree).map((b) => b.props['aria-label']);
    expect(labels).toEqual(['1-Year Subscriber', 'Moderator', '7TV Subscriber - 6 Months', 'FFZ Supporter', 'BTTV Pro']);
  });
});

describe('badge art picking', () => {
  const twitchArt = [
    { url: '1x', px: 18 },
    { url: '2x', px: 36 },
    { url: '4x', px: 72 },
  ];

  it('takes the smallest file that is sharp at the box size on this screen', () => {
    expect(pickBadgeArt(twitchArt, 18, 1)).toBe('1x');
    expect(pickBadgeArt(twitchArt, 18, 2)).toBe('2x');
    expect(pickBadgeArt(twitchArt, 72, 1)).toBe('4x');
    // Nothing big enough: the largest there is.
    expect(pickBadgeArt(twitchArt, 112, 2)).toBe('4x');
    expect(pickBadgeArt([{ url: 'only' }], 72, 1)).toBe('only');
    expect(pickBadgeArt([], 72, 1)).toBe('');
  });

  it('a third-party badge serving one file for every size keeps that file', () => {
    const tree = ChatBadgeIcons({
      twitchBadges: [],
      seventvBadge: null,
      thirdPartyBadges: [BTTV_PRO],
      streamNookMemberId: null,
    });
    buttons(tree)[0].props.onClick?.();
    expect(useBadgeSpotlightStore.getState().badge?.art).toEqual([{ url: 'https://cdn.betterttv.net/pro.png' }]);
  });
});

describe('BadgeHoverCard', () => {
  const ref: BadgeRef = {
    provider: 'moltorino',
    id: 'moltorino-supporter',
    title: 'Moltorino Supporter',
    art: [{ url: 'https://api.moltorino.com/x/3x.webp', px: 72 }],
  };

  it('shows the art, name, provider and the click hint', () => {
    const html = renderToStaticMarkup(<BadgeHoverCard badge={ref} clickable />);
    expect(html).toContain('https://api.moltorino.com/x/3x.webp');
    expect(html).toContain('Moltorino Supporter');
    expect(html).toContain('>Moltorino<');
    expect(html).toContain('Click for details');
  });

  it('drops the hint where the badge does not open anything', () => {
    const html = renderToStaticMarkup(<BadgeHoverCard badge={ref} clickable={false} />);
    expect(html).not.toContain('Click for details');
  });
});
