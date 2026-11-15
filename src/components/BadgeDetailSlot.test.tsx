// @vitest-environment happy-dom
import { act, useEffect, useMemo, useRef, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { MotionGlobalConfig } from 'framer-motion';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

MotionGlobalConfig.skipAnimations = true;
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
}));
vi.mock('../utils/platform', () => ({
  IS_MOBILE: false,
  IS_ANDROID: false,
  IS_MAC: false,
  IS_LINUX: false,
  IS_WEBKITGTK: false,
  IS_MAIN_WINDOW: true,
  isMainWindow: () => true,
}));
vi.mock('../bootPreload', () => ({ takePreloadedSettings: () => null }));
// The modules SEC-11's sharp art put in the detail body's import graph: the image draws a plain <img>,
// and nothing reaches the art store, the asset queue or the cache file server.
vi.mock('./badge/SharpBadgeImg', () => ({
  SharpBadgeImg: ({ src, alt }: { src: string; alt: string }) => <img src={src} alt={alt} />,
}));
vi.mock('../services/streamnookArt', () => ({
  getStreamnookArtVersion: () => 0,
  subscribeStreamnookArt: () => () => {},
  streamnookArtCopyOf: () => null,
  streamnookArtSrc: (url: string) => url,
  requestStreamnookArt: () => {},
}));
vi.mock('../services/assetCacheQueue', () => ({
  onAssetsCached: () => () => {},
  requestAssetCaching: () => {},
  forgetAssetRequests: () => {},
  setAssetCacheBurst: () => {},
}));
vi.mock('../utils/cacheFileUrl', () => ({
  ensureCacheFileServer: async () => {},
  awaitCacheFileServer: async () => {},
  waitsForCacheFileServer: () => false,
  isDiskCopyUrl: () => false,
  cacheFileUrl: (url: string) => url,
}));
// The Badges list is its own 2,500-line component; the slot's job is only to mount it, so a stub with
// one tile stands in for it.
vi.mock('./BadgesOverlay', () => ({
  default: ({ onBadgeClick }: { onBadgeClick: (badge: unknown, setId: string) => void }) => (
    <div data-testid="badges-overlay">
      <button data-testid="tile" onClick={() => onBadgeClick(badge('3'), 'clover')}>
        tile
      </button>
    </div>
  ),
}));

import { invoke } from '@tauri-apps/api/core';
import BadgeDetailSlot, { makeCloseLinkSheet, type SelectedBadge } from './BadgeDetailSlot';
import { openBadgeSpotlight, useBadgeSpotlightStore } from '../stores/badgeSpotlightStore';
import { BADGE_LOAD_FAILED_TEXT, BADGE_NOT_FOUND_TEXT, openBadgeLink, applyBadgeAnswer, type BadgeLinkAnswer, type BadgeLinkDeps } from '../utils/badgeLinkOpen';

function badge(id: string) {
  return {
    id,
    image_url_1x: `https://static.example/${id}/1`,
    image_url_2x: `https://static.example/${id}/2`,
    image_url_4x: `https://static.example/${id}/3`,
    title: `Clover ${id}`,
    description: 'Earned by watching.',
    click_action: null,
    click_url: null,
  };
}

interface Api {
  show: (key: string) => number;
  answer: (key: string, read: number, next: BadgeLinkAnswer) => void;
  closeLinkSheet: () => void;
  closeAppLayers: () => void;
  openLink: (setId: string, version: string) => void;
  retried: Array<{ key: string; read: number }>;
  state: () => { selected: SelectedBadge; overlay: boolean };
}

// App's badge state in miniature: the same slot, the same counter, the same close that
// makeCloseLinkSheet builds, and the real openBadgeLink and applyBadgeAnswer on top.
function Harness({ onApi, overlay = false }: { onApi: (api: Api) => void; overlay?: boolean }) {
  const [selectedBadge, setSelectedBadge] = useState<SelectedBadge>(null);
  const [showBadgesOverlay, setShowBadgesOverlay] = useState(overlay);
  const reads = useRef(0);
  const selectedRef = useRef<SelectedBadge>(null);
  const retried = useRef<Array<{ key: string; read: number }>>([]);
  useEffect(() => {
    selectedRef.current = selectedBadge;
  }, [selectedBadge]);

  const deps = useMemo<BadgeLinkDeps>(
    () => ({
      isPlayerFullscreen: () => false,
      exitPlayerFullscreen: () => {},
      showBadgeLink: (key) => {
        setShowBadgesOverlay(true);
        reads.current += 1;
        const read = reads.current;
        setSelectedBadge({ kind: 'link', key, state: 'loading', read });
        return read;
      },
      answerBadgeLink: (key, read, next) => setSelectedBadge((cur) => applyBadgeAnswer(cur, key, read, next)),
    }),
    [],
  );
  const closeLinkSheet = useMemo(() => makeCloseLinkSheet({ setSelectedBadge, setShowBadgesOverlay }), []);

  useEffect(() => {
    onApi({
      show: deps.showBadgeLink,
      answer: deps.answerBadgeLink,
      closeLinkSheet,
      closeAppLayers: () => setSelectedBadge(null),
      openLink: (setId, version) => openBadgeLink({ setId, version }, deps),
      retried: retried.current,
      state: () => ({ selected: selectedRef.current, overlay: showBadgesOverlay }),
    });
  });

  return (
    <BadgeDetailSlot
      selectedBadge={selectedBadge}
      setSelectedBadge={setSelectedBadge}
      showBadgesOverlay={showBadgesOverlay}
      setShowBadgesOverlay={setShowBadgesOverlay}
      overlayInitial={{ paintId: null, badgeId: null, streamNook: false, target: null }}
      onRetryBadgeLink={(key) => {
        const cur = selectedRef.current;
        if (cur?.kind !== 'link' || cur.key !== key || cur.state !== 'failed') return;
        reads.current += 1;
        retried.current.push({ key, read: reads.current });
        setSelectedBadge({ kind: 'link', key, state: 'loading', read: reads.current });
      }}
    />
  );
}

let host: HTMLElement;
let root: Root;
let current: Api | null = null;
const A = () => current as Api;
const q = (selector: string) => host.querySelector(selector);
const text = () => host.textContent ?? '';
const missing: BadgeLinkAnswer = { kind: 'link', key: 'clover/2', state: 'missing' };
const failed: BadgeLinkAnswer = { kind: 'link', key: 'clover/2', state: 'failed' };
const hit: BadgeLinkAnswer = { kind: 'badge', badge: badge('2'), setId: 'clover' };

// The overlays are lazy: wait, inside act, until the chunk has resolved and the selector is there.
async function until(check: () => boolean) {
  for (let i = 0; i < 100 && !check(); i += 1) {
    await act(async () => {
      await new Promise((r) => setTimeout(r, 5));
    });
  }
  expect(check()).toBe(true);
}
const act0 = (fn: () => void) => act(async () => fn());

async function mount(overlay = false) {
  await act(async () => {
    root.render(<Harness onApi={(a) => void (current = a)} overlay={overlay} />);
  });
}

// The slot loads both overlays lazily; loading the detail's module once up front keeps the first test from
// racing the cold import of its whole graph.
beforeAll(async () => {
  await import('./BadgeDetailOverlay');
}, 60_000);

beforeEach(() => {
  // A read nobody answers: each test decides what the link lookup finds by answering the slot itself.
  vi.mocked(invoke).mockReset();
  vi.mocked(invoke).mockImplementation(() => new Promise(() => {}));
  current = null;
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  useBadgeSpotlightStore.getState().close();
});

describe('BadgeDetailSlot, a badge link on its placeholder', () => {
  // Fails if the placeholder lives in BadgesOverlay (which a placeholder unmounts) or the detail waits for the read.
  it('opens the loading body in the detail frame at once, with no Badges list behind it', async () => {
    await mount();
    await act0(() => A().openLink('clover', '2'));
    await until(() => !!q('.liquid-glass-panel'));
    expect(q('[data-testid="badges-overlay"]')).toBeNull();
    expect(text()).toContain('Loading badge');
    expect(text()).toContain('Twitch Chat Badge');
    // Fails if the title row still reads a badge: the placeholder frame holds no h2.
    expect(host.querySelectorAll('h2')).toHaveLength(0);
    expect(q('button[aria-label="Back to badges"]')).not.toBeNull();
    expect(q('button[aria-label="Close"]')).not.toBeNull();
    expect(host.querySelectorAll('button')).toHaveLength(2);
  });

  // Fails if the frame unmounts and remounts when the badge arrives (a jump), or the title row is missing after it.
  it('a hit fills the same frame in place', async () => {
    await mount();
    await act0(() => A().openLink('clover', '2'));
    await until(() => !!q('.liquid-glass-panel'));
    const frame = q('.liquid-glass-panel');
    const backdrop = frame?.parentElement;
    const read = A().state().selected?.kind === 'link' ? (A().state().selected as { read: number }).read : -1;
    await act0(() => A().answer('clover/2', read, hit));
    await until(() => !!q('h2'));
    expect(q('h2')?.textContent).toBe('Clover 2');
    expect(q('.liquid-glass-panel')).toBe(frame);
    expect(q('.liquid-glass-panel')?.parentElement).toBe(backdrop);
    expect(text()).not.toContain('Loading badge');
  });

  it('a miss says so in place and offers no button', async () => {
    await mount();
    await act0(() => A().openLink('clover', '2'));
    await until(() => !!q('.liquid-glass-panel'));
    await act0(() => A().answer('clover/2', 1, missing));
    expect(text()).toContain(BADGE_NOT_FOUND_TEXT);
    expect(text()).not.toContain('Try again');
  });

  // Fails if a failed read says "not found", or the failed body has no Try again.
  it('a failed read says it could not load and offers Try again, which goes back to loading and answers again', async () => {
    await mount();
    await act0(() => A().openLink('clover', '2'));
    await until(() => !!q('.liquid-glass-panel'));
    await act0(() => A().answer('clover/2', 1, failed));
    expect(text()).toContain(BADGE_LOAD_FAILED_TEXT);
    expect(text()).not.toContain(BADGE_NOT_FOUND_TEXT);
    const button = host.querySelector('button[type="button"]') as HTMLButtonElement;
    expect(button.textContent).toBe('Try again');
    await act0(() => button.click());
    expect(A().retried).toEqual([{ key: 'clover/2', read: 2 }]);
    expect(A().state().selected).toEqual({ kind: 'link', key: 'clover/2', state: 'loading', read: 2 });
    await act0(() => A().answer('clover/2', 2, hit));
    await until(() => !!q('h2'));
    expect(q('h2')?.textContent).toBe('Clover 2');
  });

  // Fails if pressing Try again swaps the failed line for the loading body: the button would unmount and focus would be lost.
  it('Try again keeps the failed line and its button mounted, aria-disabled and focused, until the answer lands', async () => {
    await mount();
    await act0(() => A().openLink('clover', '2'));
    await until(() => !!q('.liquid-glass-panel'));
    await act0(() => A().answer('clover/2', 1, failed));
    const button = host.querySelector('button[type="button"]') as HTMLButtonElement;
    await act0(() => button.focus());
    expect(document.activeElement).toBe(button);
    await act0(() => button.click());
    const same = host.querySelector('button[type="button"]');
    expect(same).toBe(button);
    expect(button.getAttribute('aria-disabled')).toBe('true');
    expect(document.activeElement).toBe(button);
    expect(text()).toContain(BADGE_LOAD_FAILED_TEXT);
    expect(text()).not.toContain('Loading badge');
    // A second press while the read is out does nothing.
    await act0(() => button.click());
    expect(A().retried).toHaveLength(1);
    // The answer lands: a failed answer puts the button back to enabled, the same node.
    await act0(() => A().answer('clover/2', 2, failed));
    expect(host.querySelector('button[type="button"]')).toBe(button);
    expect(button.getAttribute('aria-disabled')).toBe('false');
  });

  // Fails if a retry's answer can be overwritten by the read before it.
  it('an answer for the read before a Try again writes nothing over the retry', async () => {
    await mount();
    await act0(() => A().openLink('clover', '2'));
    await until(() => !!q('.liquid-glass-panel'));
    await act0(() => A().answer('clover/2', 1, failed));
    await act0(() => (host.querySelector('button[type="button"]') as HTMLButtonElement).click());
    await act0(() => A().answer('clover/2', 1, hit));
    expect(q('h2')?.textContent).toBe('Badge link');
    expect(A().state().selected).toEqual({ kind: 'link', key: 'clover/2', state: 'loading', read: 2 });
  });

  // Fails if a late hit cannot fill a placeholder the 6 s timer already failed.
  it('a late hit for the same key and read fills a failed placeholder', async () => {
    await mount();
    await act0(() => A().openLink('clover', '2'));
    await until(() => !!q('.liquid-glass-panel'));
    await act0(() => A().answer('clover/2', 1, failed));
    await act0(() => A().answer('clover/2', 1, hit));
    await until(() => !!q('h2'));
    expect(q('h2')?.textContent).toBe('Clover 2');
  });

  // Fails if the first link's answer lands on the second link's placeholder.
  it('a second badge link during the read: the first answer writes nothing', async () => {
    await mount();
    await act0(() => A().openLink('clover', '2'));
    await act0(() => A().openLink('other', '1'));
    await until(() => !!q('.liquid-glass-panel'));
    await act0(() => A().answer('clover/2', 1, hit));
    expect(q('h2')).toBeNull();
    expect(A().state().selected).toEqual({ kind: 'link', key: 'other/1', state: 'loading', read: 2 });
  });

  // Fails if the answer reopens a detail the person closed.
  it('the person closing the detail during the read: the answer opens nothing', async () => {
    await mount();
    await act0(() => A().openLink('clover', '2'));
    await until(() => !!q('.liquid-glass-panel'));
    await act0(() => (q('button[aria-label="Close"]') as HTMLButtonElement).click());
    await until(() => !q('.liquid-glass-panel'));
    await act0(() => A().answer('clover/2', 1, hit));
    expect(q('.liquid-glass-panel')).toBeNull();
    expect(q('[data-testid="badges-overlay"]')).toBeNull();
  });

  // Fails if a category link's closeAppLayers leaves the placeholder able to reopen.
  it('a category link (closeAppLayers) before the answer: the answer mounts no detail', async () => {
    await mount();
    await act0(() => A().openLink('clover', '2'));
    await until(() => !!q('.liquid-glass-panel'));
    await act0(() => A().closeAppLayers());
    await act0(() => A().answer('clover/2', 1, hit));
    await until(() => !q('.liquid-glass-panel'));
    expect(q('.liquid-glass-panel')).toBeNull();
  });

  // Fails if the real lookup never reaches the slot: openBadgeLink, resolveBadgeLink and applyBadgeAnswer end to end.
  it('a cached hit through the real lookup fills the placeholder', async () => {
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === 'get_cached_global_badges') return { data: [{ set_id: 'clover', versions: [badge('2')] }] };
      return new Promise(() => {});
    });
    await mount();
    await act0(() => A().openLink('clover', '2'));
    await until(() => !!q('h2'));
    expect(q('h2')?.textContent).toBe('Clover 2');
  });

  it('Back from a placeholder lands on the Badges list that opened behind it', async () => {
    await mount();
    await act0(() => A().openLink('clover', '2'));
    await until(() => !!q('.liquid-glass-panel'));
    await act0(() => (q('button[aria-label="Back to badges"]') as HTMLButtonElement).click());
    await until(() => !!q('[data-testid="badges-overlay"]'));
    expect(q('.liquid-glass-panel')).toBeNull();
  });
});

describe('BadgeDetailSlot, the finished detail', () => {
  it('a tile click in the list writes a badge and mounts the detail', async () => {
    await mount(true);
    await until(() => !!q('[data-testid="tile"]'));
    await act0(() => (q('[data-testid="tile"]') as HTMLButtonElement).click());
    await until(() => !!q('h2'));
    expect(q('h2')?.textContent).toBe('Clover 3');
    expect(A().state().selected).toEqual({ kind: 'badge', badge: badge('3'), setId: 'clover' });
    expect(q('[data-testid="badges-overlay"]')).toBeNull();
  });

  it("the detail's Close clears the slot and the list, and Back clears only the slot", async () => {
    await mount(true);
    await until(() => !!q('[data-testid="tile"]'));
    await act0(() => (q('[data-testid="tile"]') as HTMLButtonElement).click());
    await until(() => !!q('h2'));
    await act0(() => (q('button[aria-label="Back to badges"]') as HTMLButtonElement).click());
    await until(() => !!q('[data-testid="badges-overlay"]'));
    await act0(() => (q('[data-testid="tile"]') as HTMLButtonElement).click());
    await until(() => !!q('h2'));
    await act0(() => (q('button[aria-label="Close"]') as HTMLButtonElement).click());
    await until(() => !q('.liquid-glass-panel'));
    expect(q('[data-testid="badges-overlay"]')).toBeNull();
    expect(A().state()).toEqual({ selected: null, overlay: false });
  });
});

describe('makeCloseLinkSheet', () => {
  // Fails with the old setSelectedBadge(null) alone: the Badges list (showBadgesOverlay still true) comes back over the new stream.
  it('a watch link after a badge link leaves neither the detail nor the Badges list mounted', async () => {
    await mount();
    await act0(() => A().openLink('clover', '2'));
    await until(() => !!q('.liquid-glass-panel'));
    expect(A().state().overlay).toBe(true);
    await act0(() => A().closeLinkSheet());
    await until(() => !q('.liquid-glass-panel'));
    expect(q('[data-testid="badges-overlay"]')).toBeNull();
    expect(A().state()).toEqual({ selected: null, overlay: false });
  });

  // Fails without the third call: a spotlight card left open would stay over the stream the watch link starts.
  it('closes the badge spotlight too', () => {
    openBadgeSpotlight({ provider: 'ffz', id: '1', title: 'Clover', art: [] });
    expect(useBadgeSpotlightStore.getState().badge).not.toBeNull();
    makeCloseLinkSheet({ setSelectedBadge: () => {}, setShowBadgesOverlay: () => {} })();
    expect(useBadgeSpotlightStore.getState().badge).toBeNull();
  });

  // Fails if either setter is not called.
  it('calls both setters', () => {
    const setSelectedBadge = vi.fn();
    const setShowBadgesOverlay = vi.fn();
    makeCloseLinkSheet({ setSelectedBadge, setShowBadgesOverlay })();
    expect(setSelectedBadge).toHaveBeenCalledWith(null);
    expect(setShowBadgesOverlay).toHaveBeenCalledWith(false);
  });
});
