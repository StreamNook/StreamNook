// @vitest-environment happy-dom
// The phone's badge link: the sheet opens at once on a placeholder that the
// read only fills, by desktop's rules (cached first, a 6 s limit, Try again,
// a late hit still fills), and every link that closes sheets closes this one,
// even before MobileSheet has pushed it.
import type { ReactNode } from 'react';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { MotionGlobalConfig } from 'framer-motion';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

MotionGlobalConfig.skipAnimations = true;
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

// Each wall read waits for the test; `onRead` runs inside the read, synchronously.
const wall = vi.hoisted(() => {
  type Read = { resolve: (value: unknown) => void; reject: (reason: unknown) => void };
  const reads: Read[] = [];
  const hooks: { onRead: (() => void) | null } = { onRead: null };
  const getBadgeWall = vi.fn((_sort: string, _force: boolean) => {
    hooks.onRead?.();
    return new Promise((resolve, reject) => {
      reads.push({ resolve, reject });
    });
  });
  return { reads, hooks, getBadgeWall };
});
const rust = vi.hoisted(() => {
  const refreshes: Array<{ resolve: (value: unknown) => void; reject: (reason: unknown) => void }> = [];
  const invoke = vi.fn((cmd: string) => {
    if (cmd === 'force_refresh_global_badges') {
      return new Promise((resolve, reject) => {
        refreshes.push({ resolve, reject });
      });
    }
    return Promise.resolve(null);
  });
  return { refreshes, invoke };
});
const spies = vi.hoisted(() => ({ toast: vi.fn(), loadGallery: vi.fn(async (_sort: string, _force: boolean) => {}) }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: rust.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('../../stores/AppStore', () => {
  const state = {
    addToast: spies.toast,
    currentUser: { user_id: '1' },
    startStream: vi.fn(),
    isAuthenticated: true,
    streamUrl: null,
    isLoading: false,
  };
  return {
    useAppStore: Object.assign((select: (s: typeof state) => unknown) => select(state), { getState: () => state }),
  };
});
vi.mock('../../stores/badgeSpotlightStore', () => ({
  useBadgeSpotlightStore: { getState: () => ({ close: () => {} }) },
}));
vi.mock('../phonePrefs', () => ({ startTab: () => 'following' }));
vi.mock('../../utils/logger', () => ({ Logger: { debug: vi.fn(), info: vi.fn(), warn: vi.fn(), error: vi.fn() } }));
vi.mock('../rewards/rewardsModel', () => ({
  getBadgeWall: wall.getBadgeWall,
  getRewardsDrops: vi.fn(async () => ({
    connected: true,
    query: '',
    in_progress: [],
    available: [],
    earned: [],
    in_progress_empty: '',
    no_match: '',
  })),
  blurbText: () => '',
}));
// Only the link reads the wall here: the gallery store is a stub, so its own reads never reach getBadgeWall.
vi.mock('../rewards/badgeGalleryStore', () => ({
  useBadgeGallery: Object.assign(
    (select: (s: { wall: null; loading: boolean; syncLeft: number }) => unknown) =>
      select({ wall: null, loading: false, syncLeft: 0 }),
    { getState: () => ({ wall: null }) },
  ),
  badgeGalleryIsWarm: () => false,
  followBadgeSync: () => {},
  loadBadgeGallery: spies.loadGallery,
}));
vi.mock('../rewards/MobileMissingNow', () => ({ MobileMissingNow: () => null }));
vi.mock('../rewards/MobileEvolution', () => ({ MobileEvolution: () => null }));
vi.mock('../rewards/BadgeSortMenu', () => ({ BadgeSortMenu: () => null }));
vi.mock('../rewards/WatchPlaceCards', () => ({ PlaceCategoryCard: () => null, PlaceChannelCard: () => null }));
vi.mock('../ui/ScreenHeader', () => ({ ScreenHeader: ({ children }: { children: ReactNode }) => <div>{children}</div> }));
vi.mock('../ui/TitleControls', () => ({ TitleSegments: () => null }));
vi.mock('../ui/PullToRefresh', () => ({ PullToRefresh: ({ children }: { children: ReactNode }) => <div>{children}</div> }));
vi.mock('../ui/SearchField', () => ({ SearchField: () => null }));
vi.mock('../ui/SettleIn', () => ({
  SettleIn: ({ children }: { children: ReactNode }) => <>{children}</>,
  useSettleIn: () => true,
}));
vi.mock('../../components/ProviderLogo', () => ({ ProviderMark: () => null }));
vi.mock('../../components/badge/SharpBadgeImg', () => ({
  SharpBadgeImg: ({ src, className }: { src: string; className?: string }) => <img src={src} alt="" className={className} />,
}));
vi.mock('../../services/badgeStanding', () => ({ getBadgeEarnGuide: () => new Promise(() => {}) }));
vi.mock('../../services/watchPlaces', () => ({ resolveWatchPlaces: () => new Promise(() => {}) }));

import { RewardsScreen } from './RewardsScreen';
import { useMobileNavStore } from '../navStore';
import { BADGE_LOAD_FAILED_TEXT, BADGE_NOT_FOUND_TEXT } from '../../utils/badgeLinkOpen';

const tile = (key: string, title: string) => ({
  key,
  keys: [key],
  title,
  image: `https://img.test/${key}`,
  owned: false,
  status: null,
  word: '',
  sheet_word: '',
  prose: [],
  window: [],
  caveat: null,
  usage_label: null,
  info_url: null,
  added_label: null,
});
const loadedWall = (tiles: unknown[]) => ({
  loading: false,
  tiles,
  count_label: '',
  sync_left: 0,
  missing: { state: 'hidden', refreshing: false, closes: '', catch_up: null, rows: [] },
  next_change_ms: null,
});
const OFFLINE = { loading: true, tiles: [] };

let root: Root;
let host: HTMLDivElement;

const flush = () =>
  act(async () => {
    await new Promise((done) => setTimeout(done, 0));
  });
const artBox = () => document.querySelector('[data-testid="badge-art-box"]');
const slot = () => document.querySelector('[data-testid="badge-title-slot"]');
const loadingBody = () => slot()?.getAttribute('aria-hidden') === 'true';
const tryAgain = () =>
  [...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent === 'Try again');
const tileShown = (title: string) =>
  [...document.querySelectorAll('div')].some((d) => d.textContent === title && d.className.includes('text-[16px]'));

async function mount() {
  await act(async () => root.render(<RewardsScreen />));
  await flush();
}

async function openLink(key: string) {
  await act(async () => {
    useMobileNavStore.getState().openBadgeLink(key);
  });
  await flush();
}

async function settleRead(index: number, value: unknown) {
  await act(async () => {
    wall.reads[index].resolve(value);
  });
  await flush();
}

async function failRead(index: number) {
  await act(async () => {
    wall.reads[index].reject(new Error('offline'));
  });
  await flush();
}

async function settleRefresh(index: number, ok: boolean) {
  await act(async () => {
    if (ok) rust.refreshes[index].resolve({ data: [] });
    else rust.refreshes[index].reject(new Error('refused'));
  });
  await flush();
}

async function pressTryAgain() {
  const button = tryAgain();
  if (!button) throw new Error('no Try again');
  button.focus();
  await act(async () => {
    button.click();
  });
  await flush();
  return button;
}

beforeEach(() => {
  wall.reads.length = 0;
  wall.hooks.onRead = null;
  wall.getBadgeWall.mockClear();
  rust.refreshes.length = 0;
  spies.toast.mockClear();
  spies.loadGallery.mockClear();
  useMobileNavStore.setState({ sheetStack: [], badgeLinkKey: null, linkSheetSeq: 0, browseCategory: null });
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(async () => {
  vi.useRealTimers();
  await act(async () => root.unmount());
  host.remove();
});

const plainReads = () => spies.loadGallery.mock.calls.filter((call) => call[1] === false).length;

describe('a badge link opens at once', () => {
  // Fails if the sheet waits for the read, or the placeholder and the tile do not share the art box and title line.
  it('shows the placeholder at once, and a cached hit fills it in place with no refresh and no toast', async () => {
    await mount();
    await openLink('clover/3');
    expect(artBox()?.className).toContain('w-20 h-20 mb-2.5');
    expect(loadingBody()).toBe(true);
    expect(wall.getBadgeWall).toHaveBeenCalledWith('newest', false);
    await settleRead(0, loadedWall([tile('clover/3', 'Clover')]));
    expect(artBox()).toBeNull();
    expect(document.querySelector('img[src="https://img.test/clover/3"]')?.className).toContain('w-20 h-20');
    expect(document.querySelector('img[src="https://img.test/clover/3"]')?.className).toContain('mb-2.5');
    expect(tileShown('Clover')).toBe(true);
    expect(rust.refreshes).toHaveLength(0);
    expect(spies.toast).not.toHaveBeenCalled();
    expect(plainReads()).toBe(1);
  });

  // Fails if a miss after the refresh shows a toast or a Try again that cannot help.
  it('a miss, then a refresh, then a miss says not found with no Try again', async () => {
    await mount();
    await openLink('clover/3');
    await settleRead(0, loadedWall([]));
    await settleRefresh(0, true);
    await settleRead(1, loadedWall([]));
    expect(slot()?.textContent).toBe(BADGE_NOT_FOUND_TEXT);
    expect(tryAgain()).toBeUndefined();
    expect(spies.toast).not.toHaveBeenCalled();
  });

  // Fails if offline reads as "not found", or a refused refresh with a loaded cache reads as failed.
  it('a refused refresh answers from the cached wall: offline is failed, a loaded wall without it is missing', async () => {
    await mount();
    await openLink('clover/3');
    await settleRead(0, OFFLINE);
    await settleRefresh(0, false);
    expect(slot()?.textContent).toBe(BADGE_LOAD_FAILED_TEXT);
    expect(tryAgain()).toBeDefined();
    await openLink('other/1');
    await settleRead(1, loadedWall([tile('clover/3', 'Clover')]));
    await settleRefresh(1, false);
    expect(slot()?.textContent).toBe(BADGE_NOT_FOUND_TEXT);
  });

  // Fails if Try again loses focus, accepts a second press, or its answer cannot fill the sheet.
  it('a thrown read fails with Try again, which reads again, keeps focus and fills in place', async () => {
    await mount();
    await openLink('clover/3');
    await failRead(0);
    expect(slot()?.textContent).toBe(BADGE_LOAD_FAILED_TEXT);
    const button = await pressTryAgain();
    expect(wall.getBadgeWall).toHaveBeenCalledTimes(2);
    expect(tryAgain()).toBe(button);
    expect(button.getAttribute('aria-disabled')).toBe('true');
    expect(document.activeElement).toBe(button);
    await settleRead(1, loadedWall([tile('clover/3', 'Clover')]));
    expect(tileShown('Clover')).toBe(true);
  });

  // Fails if a retry's answer opens a sheet the person closed meanwhile.
  it('a retry answer after the sheet was closed writes nothing', async () => {
    await mount();
    await openLink('clover/3');
    await failRead(0);
    await pressTryAgain();
    await act(async () => {
      useMobileNavStore.getState().closeLinkSheet();
    });
    await flush();
    await settleRead(1, loadedWall([tile('clover/3', 'Clover')]));
    expect(artBox()).toBeNull();
    expect(tileShown('Clover')).toBe(false);
  });

  // Fails if a new link inherits the Try again press made on the sheet before it.
  it('a new link after a Try again shows the loading body, for another key and for the same key', async () => {
    await mount();
    await openLink('clover/3');
    await failRead(0);
    await pressTryAgain();
    await openLink('other/1');
    expect(loadingBody()).toBe(true);
    expect(tryAgain()).toBeUndefined();
    await failRead(2);
    await pressTryAgain();
    await openLink('other/1');
    expect(loadingBody()).toBe(true);
    expect(tryAgain()).toBeUndefined();
  });
});

describe('the 6 s limit', () => {
  const tick = (ms: number) =>
    act(async () => {
      await vi.advanceTimersByTimeAsync(ms);
    });

  // Fails without the limit, or if the timer refreshes the wall or cancels the read.
  it('a read still pending at 6 s fails the sheet; a late miss writes nothing and a late hit fills it', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    await act(async () => root.render(<RewardsScreen />));
    await tick(0);
    await act(async () => {
      useMobileNavStore.getState().openBadgeLink('clover/3');
    });
    await tick(5999);
    expect(loadingBody()).toBe(true);
    await tick(1);
    expect(slot()?.textContent).toBe(BADGE_LOAD_FAILED_TEXT);
    expect(plainReads()).toBe(0);
    await act(async () => {
      wall.reads[0].resolve(loadedWall([]));
    });
    await tick(0);
    await act(async () => {
      rust.refreshes[0].resolve({ data: [] });
    });
    await tick(0);
    await act(async () => {
      wall.reads[1].resolve(loadedWall([]));
    });
    await tick(0);
    expect(slot()?.textContent).toBe(BADGE_LOAD_FAILED_TEXT);
    expect(plainReads()).toBe(1);
    await act(async () => {
      useMobileNavStore.getState().openBadgeLink('other/1');
    });
    await tick(6000);
    expect(slot()?.textContent).toBe(BADGE_LOAD_FAILED_TEXT);
    await act(async () => {
      wall.reads[2].resolve(loadedWall([tile('other/1', 'Other')]));
    });
    await tick(0);
    expect(tileShown('Other')).toBe(true);
  });
});

describe('links that close sheets close this one', () => {
  // Fails without the linkSheetSeq effect: the walk finds an empty stack and the taken sheet stays.
  it('closes a sheet taken from a link before MobileSheet pushed it', async () => {
    await mount();
    let stackAtRead = -1;
    wall.hooks.onRead = () => {
      wall.hooks.onRead = null;
      stackAtRead = useMobileNavStore.getState().sheetStack.length;
      useMobileNavStore.getState().closeLinkSheet();
    };
    await openLink('clover/3');
    expect(stackAtRead).toBe(0);
    expect(artBox()).toBeNull();
    await settleRead(0, loadedWall([tile('clover/3', 'Clover')]));
    expect(tileShown('Clover')).toBe(false);
  });

  // Fails when the seq effect is declared after the link effect: the bump would close the new sheet.
  it('a close and a badge link in one block still show the new badge', async () => {
    await mount();
    await act(async () => {
      useMobileNavStore.getState().closeLinkSheet();
      useMobileNavStore.getState().openBadgeLink('b/1');
    });
    await flush();
    expect(artBox()).not.toBeNull();
    expect(loadingBody()).toBe(true);
  });

  // Fails if a watch link's close leaves the sheet, or the read's late answer reopens it.
  it('a watch link during the read closes the sheet, and the answer writes nothing', async () => {
    await mount();
    await openLink('clover/3');
    expect(useMobileNavStore.getState().sheetStack.length).toBe(1);
    await act(async () => {
      useMobileNavStore.getState().closeLinkSheet();
    });
    await flush();
    expect(artBox()).toBeNull();
    await settleRead(0, loadedWall([tile('clover/3', 'Clover')]));
    expect(tileShown('Clover')).toBe(false);
  });

  // Fails if a badge key left by an earlier link outlives a later category link.
  it('opens nothing on mount after a badge link that a category link replaced', async () => {
    await act(async () => {
      useMobileNavStore.getState().openBadgeLink('a/1');
      useMobileNavStore.getState().openCategoryLink('509658');
    });
    await mount();
    expect(artBox()).toBeNull();
    expect(wall.getBadgeWall).not.toHaveBeenCalled();
  });
});
