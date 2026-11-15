import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

// The store's door for a category link: it opens through the in-app category
// path (so full screen, compact view and layers behave as for a click), opens
// at once with no name, and names the copies it holds when an answer lands,
// early or late.

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(),
}));
vi.mock('../bootPreload', () => ({ takePreloadedSettings: () => null }));

const leave = vi.hoisted(() => vi.fn());
vi.mock('../utils/windowFullscreen', async (original) => ({
  ...(await original<typeof import('../utils/windowFullscreen')>()),
  exitPlayerFullscreen: leave,
}));

const activity = vi.hoisted(() => vi.fn());
vi.mock('../services/logService', async (original) => ({
  ...(await original<typeof import('../services/logService')>()),
  trackActivity: activity,
}));

import { useAppStore } from './AppStore';
import { registerPlayerControls, type PlayerControls } from '../keybindings/playerControls';
import { getLinkedCategory, resetLinkedCategories, resolveLinkedCategory } from '../utils/linkedCategory';

const get = () => useAppStore.getState();
// Tests swap actions for spies; every case starts from the real ones.
const real = {
  openStreamCategory: get().openStreamCategory,
  closeLayersOverHome: get().closeLayersOverHome,
  toggleTheaterMode: get().toggleTheaterMode,
};
const calls = (cmd: string) => invokeMock.mock.calls.filter(([c]) => c === cmd);
const bare = (id: string) => ({ id, name: '', box_art_url: '' });
const answer = (id: string, name: string) => ({
  state: 'found',
  id,
  name,
  box_art_url: `https://img.example/${name}.jpg`,
  viewer_count: 12,
});

function playerWith(toggleFullscreen: () => void): PlayerControls {
  return {
    isActive: () => true,
    getCurrentTime: () => 0,
    togglePlay: () => {},
    toggleMute: () => {},
    toggleFullscreen,
    volumeUp: () => {},
    volumeDown: () => {},
    seekForward: () => {},
    seekBackward: () => {},
    togglePip: () => {},
    speedUp: () => {},
    speedDown: () => {},
    goLive: () => {},
  };
}

beforeEach(() => {
  vi.useFakeTimers();
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
  leave.mockReset();
  activity.mockReset();
  resetLinkedCategories();
  registerPlayerControls(null);
  useAppStore.setState({
    ...real,
    isPlayerFullscreen: false,
    isTheaterMode: false,
    isHomeActive: false,
    homeActiveTab: 'following',
    homeCategoryTab: 'clips',
    homeSelectedCategory: null,
    homeLastExitedCategory: null,
    streamOriginCategory: null,
    homeProviderCategory: null,
    activePlatform: 'all',
    showBadgesOverlay: false,
    showDropsOverlay: false,
    currentStream: null,
    streamUrl: null,
  });
});

afterEach(() => {
  registerPlayerControls(null);
  vi.useRealTimers();
});

describe('openStreamCategory and full screen', () => {
  it('leaves full screen through the window exit, once, and never toggles it', () => {
    // Fails against the old store, which called the player's toggleFullscreen:
    // that misses a MultiNook tile and re-enters full screen when the exit has
    // just run and the store flag has not caught up.
    const toggle = vi.fn();
    registerPlayerControls(playerWith(toggle));
    useAppStore.setState({ isPlayerFullscreen: true });
    get().openStreamCategory({ kind: 'twitch', category: { id: '509658', name: 'Just Chatting', box_art_url: '' } });
    expect(leave).toHaveBeenCalledTimes(1);
    expect(toggle).not.toHaveBeenCalled();
  });

  it('leaves the window alone when the player is not in full screen', () => {
    const toggle = vi.fn();
    registerPlayerControls(playerWith(toggle));
    get().openStreamCategory({ kind: 'twitch', category: { id: '509658', name: 'Just Chatting', box_art_url: '' } });
    expect(leave).not.toHaveBeenCalled();
    expect(toggle).not.toHaveBeenCalled();
  });

  it('logs a nameless category by its id', () => {
    // Fails against the old store, which logged "Opened category: " with nothing after it.
    get().openStreamCategory({ kind: 'twitch', category: bare('509658') });
    expect(activity).toHaveBeenCalledWith('Opened category: 509658');
    get().openStreamCategory({ kind: 'twitch', category: { id: '1', name: 'Rust', box_art_url: '' } });
    expect(activity).toHaveBeenCalledWith('Opened category: Rust');
  });
});

describe('openCategoryFromLink', () => {
  it('opens the nameless category synchronously, on its Live tab, with Home up', () => {
    // Fails against the old store: the action does not exist.
    get().openCategoryFromLink('509658');
    expect(get().homeSelectedCategory).toEqual(bare('509658'));
    expect(get().homeActiveTab).toBe('category');
    expect(get().homeCategoryTab).toBe('live');
    expect(get().isHomeActive).toBe(true);
  });

  it('opens through openStreamCategory, once, with the nameless copy', () => {
    const open = vi.fn();
    useAppStore.setState({ openStreamCategory: open });
    get().openCategoryFromLink('509658');
    expect(open).toHaveBeenCalledTimes(1);
    expect(open).toHaveBeenCalledWith({ kind: 'twitch', category: bare('509658') });
  });

  it('closes the layers over Home once, before it opens the category', () => {
    const order: string[] = [];
    useAppStore.setState({
      closeLayersOverHome: () => void order.push('layers'),
      openStreamCategory: () => void order.push('open'),
    });
    get().openCategoryFromLink('509658');
    expect(order).toEqual(['layers', 'open']);
  });

  it('leaves compact view once, and only when the player is in it', () => {
    const toggle = vi.fn();
    useAppStore.setState({ isTheaterMode: true, toggleTheaterMode: toggle });
    get().openCategoryFromLink('509658');
    expect(toggle).toHaveBeenCalledTimes(1);
    toggle.mockClear();
    useAppStore.setState({ isTheaterMode: false });
    get().openCategoryFromLink('509658');
    expect(toggle).not.toHaveBeenCalled();
  });

  it('keeps the name of the category already shown', () => {
    const shown = { id: '509658', name: 'Just Chatting', box_art_url: 'https://img.example/jc.jpg' };
    useAppStore.setState({ homeSelectedCategory: shown, homeActiveTab: 'browse' });
    get().openCategoryFromLink('509658');
    expect(get().homeSelectedCategory).toBe(shown);
    expect(get().homeActiveTab).toBe('category');
  });

  it('replaces another category with the nameless one', () => {
    useAppStore.setState({ homeSelectedCategory: { id: '1', name: 'Rust', box_art_url: '' } });
    get().openCategoryFromLink('509658');
    expect(get().homeSelectedCategory).toEqual(bare('509658'));
  });

  it('opens already named when an answer is stored, before any promise settles', async () => {
    // Fails if the name waits for Home's own resolve: the header would show its
    // loading state for a frame it did not need to.
    invokeMock.mockResolvedValue(answer('509658', 'Just Chatting'));
    await resolveLinkedCategory('509658');
    expect(getLinkedCategory('509658')?.state).toBe('found');
    get().openCategoryFromLink('509658');
    expect(get().homeSelectedCategory).toEqual({
      id: '509658',
      name: 'Just Chatting',
      box_art_url: 'https://img.example/Just Chatting.jpg',
    });
  });

  it('drops a failed answer for the id, so the link is a fresh attempt', async () => {
    invokeMock.mockRejectedValue(new Error('offline'));
    await resolveLinkedCategory('509658');
    expect(getLinkedCategory('509658')?.state).toBe('failed');
    get().openCategoryFromLink('509658');
    expect(getLinkedCategory('509658')).toBeUndefined();
  });

  it('is built on openStreamCategory and never writes the screen itself (source pin)', () => {
    const body = actionBody('openCategoryFromLink');
    expect(body).toContain('openStreamCategory(');
    expect(body).not.toContain('isHomeActive');
    expect(body).not.toContain('homeActiveTab');
  });
});

describe('a link starts nothing and stops nothing', () => {
  it('lets a restart in flight apply its answer and clear its flag (guard, not coverage)', async () => {
    // The link opens through the in-app category path and touches no start,
    // stop or Home writer, so a restart that is waiting on Rust is not
    // superseded by it. This case fails if the link path ever claims a start
    // sequence or sends a stop of its own.
    let release: () => void = () => {};
    const gate = new Promise<void>((r) => {
      release = r;
    });
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'start_stream') {
        await gate;
        return { url: 'http://localhost/relay-1.m3u8', quality: 'best' };
      }
      return undefined;
    });
    useAppStore.setState({
      currentStream: { user_login: 'chan', user_name: 'Chan', user_id: '1' } as never,
      currentMediaType: 'live',
      settings: { quality: 'best' } as never,
      streamUrl: 'http://localhost/old.m3u8',
      isRestartingStream: false,
      addToast: vi.fn() as never,
    });
    const restart = get().restartStream({ quiet: true });
    await vi.advanceTimersByTimeAsync(1000);
    expect(calls('start_stream')).toHaveLength(1);
    const stopsBefore = calls('stop_stream').length;

    get().openCategoryFromLink('509658');
    release();
    await vi.advanceTimersByTimeAsync(1000);
    await restart;

    expect(get().streamUrl).toBe('http://localhost/relay-1.m3u8');
    expect(get().isRestartingStream).toBe(false);
    expect(calls('watch_stop')).toHaveLength(0);
    expect(calls('stop_stream')).toHaveLength(stopsBefore);
  });
});

describe('fillLinkedCategory', () => {
  const named = { id: '1', name: 'Rust', box_art_url: 'https://img.example/rust.jpg' };

  it('names each nameless copy of the id in one write', () => {
    // Fails against the old store: the action does not exist.
    useAppStore.setState({
      homeSelectedCategory: bare('1'),
      homeLastExitedCategory: bare('1'),
      streamOriginCategory: bare('1'),
    });
    const heard = vi.fn();
    const stop = useAppStore.subscribe(heard);
    get().fillLinkedCategory('1', named);
    stop();
    expect(get().homeSelectedCategory).toEqual(named);
    expect(get().homeLastExitedCategory).toEqual(named);
    expect(get().streamOriginCategory).toEqual(named);
    expect(heard).toHaveBeenCalledTimes(1);
  });

  it('leaves another id and an already named copy alone, and writes nothing', () => {
    const other = bare('2');
    const already = { id: '1', name: 'Already', box_art_url: '' };
    useAppStore.setState({ homeSelectedCategory: other, homeLastExitedCategory: already, streamOriginCategory: null });
    const heard = vi.fn();
    const stop = useAppStore.subscribe(heard);
    get().fillLinkedCategory('1', named);
    stop();
    expect(get().homeSelectedCategory).toBe(other);
    expect(get().homeLastExitedCategory).toBe(already);
    expect(heard).not.toHaveBeenCalled();
  });

  it('touches no tab, no Home flag and no layer', () => {
    useAppStore.setState({
      homeSelectedCategory: bare('1'),
      homeActiveTab: 'browse',
      isHomeActive: false,
      showBadgesOverlay: true,
      showDropsOverlay: true,
      isSettingsOpen: true,
    });
    get().fillLinkedCategory('1', named);
    expect(get().homeActiveTab).toBe('browse');
    expect(get().isHomeActive).toBe(false);
    expect(get().showBadgesOverlay).toBe(true);
    expect(get().showDropsOverlay).toBe(true);
    expect(get().isSettingsOpen).toBe(true);
  });

  it('after Back, an answer still names the copy that Forward re-enters', () => {
    // Fails if the fill goes through setHomeSelectedCategory or skips
    // homeLastExitedCategory: Forward would bring back a nameless header.
    get().openCategoryFromLink('1');
    get().navigateBack();
    expect(get().homeSelectedCategory).toBeNull();
    get().fillLinkedCategory('1', named);
    get().navigateForward();
    expect(get().homeSelectedCategory).toEqual(named);
  });

  it('names the category the player came from, so its Back lands on a named one', () => {
    useAppStore.setState({ streamOriginCategory: bare('1') });
    get().fillLinkedCategory('1', named);
    expect(get().streamOriginCategory).toEqual(named);
  });

  it('an answer for the first of two links leaves the second one nameless', () => {
    get().openCategoryFromLink('1');
    get().openCategoryFromLink('2');
    get().fillLinkedCategory('1', named);
    expect(get().homeSelectedCategory).toEqual(bare('2'));
  });

  it('never goes through setHomeSelectedCategory (source pin)', () => {
    // That setter drops the forward step, which is one of the copies this fills.
    const body = actionBody('fillLinkedCategory');
    expect(body).not.toContain('setHomeSelectedCategory');
    expect(body.match(/\bset\(/g)).toHaveLength(1);
  });
});

describe('a late answer reaches the store without any screen asking', () => {
  it('names the selection and the origin when a found replaces a failed record', async () => {
    // Fails if the store never registers with onLateLinkedCategory, or if
    // resetLinkedCategories dropped that registration: the copies stay nameless.
    let late: (v: unknown) => void = () => {};
    invokeMock.mockImplementation(
      (cmd: string) =>
        cmd === 'resolve_category_link'
          ? new Promise((r) => {
              late = r;
            })
          : Promise.resolve(undefined),
    );
    get().openCategoryFromLink('509658');
    useAppStore.setState({ streamOriginCategory: bare('509658') });
    void resolveLinkedCategory('509658');
    await vi.advanceTimersByTimeAsync(6000);
    expect(getLinkedCategory('509658')?.state).toBe('failed');
    expect(get().homeSelectedCategory?.name).toBe('');

    late(answer('509658', 'Fresh Name'));
    await vi.advanceTimersByTimeAsync(2000);

    expect(getLinkedCategory('509658')?.state).toBe('found');
    expect(get().homeSelectedCategory?.name).toBe('Fresh Name');
    expect(get().streamOriginCategory?.name).toBe('Fresh Name');
  });
});

/** The text of one store action, from its key to the next one at the same indent. */
function actionBody(name: string): string {
  const text = readFileSync(resolve(__dirname, 'AppStore.ts'), 'utf8').replace(/\r\n/g, '\n');
  // The interface declares each action first; the implementation is the last match.
  const start = text.lastIndexOf(`\n  ${name}: (`);
  expect(start).toBeGreaterThan(-1);
  const end = text.indexOf('\n  },\n', start);
  expect(end).toBeGreaterThan(start);
  return text.slice(start, end);
}
