// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { MotionGlobalConfig } from 'framer-motion';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import type { TwitchCategory, TwitchStream } from '../types';

// Motion finishes at once and React treats this as a test environment, so
// every render below settles inside its own act.
MotionGlobalConfig.skipAnimations = true;
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

// Every category call is held until the test answers it, so a late answer for
// an older category is something a test can send on purpose.
const rust = vi.hoisted(() => {
  type Call = {
    cmd: string;
    args: Record<string, unknown> | undefined;
    settled: boolean;
    resolve: (value: unknown) => void;
    reject: (reason: unknown) => void;
  };
  const calls: Call[] = [];
  const HELD = new Set([
    'get_streams_by_game',
    'get_streams_by_game_name',
    'get_streams_by_game_with_tags',
    'get_category_info',
    'get_clips_by_game',
    'get_videos_by_game',
    'category_share_link',
    // Typing in the filter box asks for tag suggestions; an answer is not needed here.
    'category_tag_suggestions',
  ]);
  const invoke = (cmd: string, args?: Record<string, unknown>) => {
    if (!HELD.has(cmd)) return Promise.resolve(null);
    return new Promise((resolve, reject) => {
      calls.push({ cmd, args, settled: false, resolve, reject });
    });
  };
  return { calls, invoke };
});

vi.mock('@tauri-apps/api/core', () => ({
  invoke: rust.invoke,
  convertFileSrc: (path: string) => path,
}));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
}));
// Explicit flags, and the two main-window names noticeReport.ts reads.
vi.mock('../utils/platform', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  IS_MOBILE: false,
  IS_LINUX: false,
  IS_MAIN_WINDOW: true,
  isMainWindow: () => true,
}));

// The linked-category records are driven by hand: this file tests what Home
// does with a record, not how the record is made. Every other export stays real.
const records = vi.hoisted(() => {
  type Rec = { state: 'loading' | 'found' | 'failed' | 'missing'; category?: TwitchCategory };
  type Ask = { id: string; retry: boolean; settle: (rec: Rec) => void };
  const map = new Map<string, Rec>();
  const listeners = new Set<() => void>();
  const asks: Ask[] = [];
  // A resolve already in flight for an id is shared, as the real module does.
  const inFlight = new Map<string, Promise<Rec>>();
  const put = (id: string, rec: Rec | null) => {
    if (rec) map.set(id, rec);
    else map.delete(id);
    listeners.forEach((l) => l());
  };
  const resolve = (id: string, opts?: { retry?: boolean }) => {
    const retry = !!opts?.retry;
    const known = map.get(id);
    if (known && !retry && known.state !== 'loading') return Promise.resolve(known);
    const shared = inFlight.get(id);
    if (shared && !retry) return shared;
    if (retry) put(id, { state: 'loading' });
    const ask = new Promise<Rec>((done) => {
      asks.push({
        id,
        retry,
        settle: (rec) => {
          inFlight.delete(id);
          put(id, rec);
          done(rec);
        },
      });
    });
    inFlight.set(id, ask);
    return ask;
  };
  return { map, listeners, asks, inFlight, put, resolve };
});
vi.mock('../utils/linkedCategory', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../utils/linkedCategory')>();
  const { useSyncExternalStore } = await import('react');
  return {
    ...actual,
    resolveLinkedCategory: records.resolve,
    useLinkedCategory: (id: string | null) =>
      useSyncExternalStore(
        (listener: () => void) => {
          records.listeners.add(listener);
          return () => records.listeners.delete(listener);
        },
        () => (id ? records.map.get(id) : undefined),
      ),
  };
});

import Home from './Home';
import { useAppStore } from '../stores/AppStore';

const initialState = useAppStore.getState();

const NAMELESS: TwitchCategory = { id: '509658', name: '', box_art_url: '' };
const RUST: TwitchCategory = { id: '509658', name: 'Rust', box_art_url: '' };
const ALPHA: TwitchCategory = { id: '1', name: 'Alpha', box_art_url: '' };
const BETA: TwitchCategory = { id: '2', name: 'Beta', box_art_url: '' };

const stream = (id: string, login: string): TwitchStream => ({
  id,
  user_id: `u${id}`,
  user_name: login,
  user_login: login,
  title: `${login} title`,
  viewer_count: 10,
  game_id: '1',
  game_name: 'Alpha',
  thumbnail_url: '',
  started_at: '2026-01-01T00:00:00Z',
});

const info = (name: string, over: Record<string, unknown> = {}) => ({
  id: '509658',
  name,
  displayName: name,
  description: null,
  followersCount: null,
  boxArtUrl: null,
  tags: null,
  ...over,
});

const callsOf = (cmd: string) => rust.calls.filter((c) => c.cmd === cmd);
const lastCall = (cmd: string, match?: (args: Record<string, unknown> | undefined) => boolean) => {
  const found = callsOf(cmd).filter((c) => !c.settled && (!match || match(c.args)));
  const call = found[found.length - 1];
  if (!call) throw new Error(`no pending ${cmd} call`);
  return call;
};
const answer = async (call: ReturnType<typeof lastCall>, value: unknown) => {
  call.settled = true;
  await act(async () => {
    call.resolve(value);
    await Promise.resolve();
  });
};
const refuse = async (call: ReturnType<typeof lastCall>, reason: unknown) => {
  call.settled = true;
  await act(async () => {
    call.reject(reason);
    await Promise.resolve();
  });
};
const forId = (id: string) => (args: Record<string, unknown> | undefined) => args?.gameId === id;
const forName = (name: string) => (args: Record<string, unknown> | undefined) => args?.gameName === name;

// React's own complaints (act, nesting, keys) are test failures, not noise.
const reactWarnings: string[] = [];
let consoleError: ReturnType<typeof vi.spyOn>;

let root: Root | null = null;
let host: HTMLElement;

const mount = async () => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  await act(async () => {
    root!.render(<Home />);
  });
};
const setStore = async (patch: Record<string, unknown>) => {
  await act(async () => {
    useAppStore.setState(patch as never);
  });
};
const open = (category: TwitchCategory) => setStore({ homeSelectedCategory: category, homeActiveTab: 'category', isHomeActive: true });
const text = () => host.textContent ?? '';
const titleBar = () => host.querySelector('h1');
const searchBox = () => host.querySelector('.max-w-\\[460px\\] input');
const buttonNamed = (name: string) =>
  Array.from(host.querySelectorAll('button')).find((b) => b.textContent?.trim() === name) as HTMLButtonElement | undefined;
const click = async (el: Element | null | undefined) => {
  if (!el) throw new Error('nothing to click');
  await act(async () => {
    el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }));
  });
};

beforeEach(() => {
  rust.calls.length = 0;
  records.map.clear();
  records.asks.length = 0;
  records.inFlight.clear();
  reactWarnings.length = 0;
  useAppStore.setState(initialState, true);
  consoleError = vi.spyOn(console, 'error').mockImplementation((...args: unknown[]) => {
    const first = String(args[0] ?? '');
    if (/act\(|validateDOMNesting|Each child in a list|Cannot update a component|Encountered two children/.test(first)) {
      reactWarnings.push(first);
    }
  });
});

afterEach(async () => {
  if (root) {
    await act(async () => {
      root!.unmount();
    });
    root = null;
  }
  host?.remove();
  consoleError.mockRestore();
  expect(reactWarnings).toEqual([]);
  vi.useRealTimers();
});

describe('Home with a category that has no name yet', () => {
  it('shows the skeleton title, loads streams by id and asks for no details or tags', async () => {
    await setStore({ homeSelectedCategory: NAMELESS, homeActiveTab: 'category', isHomeActive: true });
    await mount();

    const h1 = titleBar();
    expect(h1).not.toBeNull();
    // Not an empty heading: the bar is hidden from screen readers, the words are not.
    expect(h1!.textContent).toBe('Loading category');
    const bar = h1!.querySelector('[aria-hidden="true"]');
    expect(bar?.className).toContain('animate-pulse');
    expect(bar?.className).toContain('h-8');

    expect(callsOf('get_streams_by_game').map((c) => c.args?.gameId)).toEqual(['509658']);
    // Without the empty-name guard Home asks for the details of a category named ''.
    expect(callsOf('get_category_info')).toEqual([]);
    expect(callsOf('get_streams_by_game_with_tags')).toEqual([]);
  });

  it('has no favorite, share or search box, and the art frame pulses with the grid icon', async () => {
    await setStore({ homeSelectedCategory: NAMELESS, homeActiveTab: 'category', isHomeActive: true });
    await mount();
    await answer(lastCall('get_streams_by_game'), [[], null]);

    expect(buttonNamed('Favorite')).toBeUndefined();
    expect(host.querySelector('[aria-label="Share category"]')).toBeNull();
    expect(searchBox()).toBeNull();
    // The frame holds the LayoutGrid icon (no art yet) and pulses like the tag chips.
    expect(host.querySelectorAll('.animate-pulse').length).toBeGreaterThanOrEqual(4);
    expect(host.querySelector('.rounded-xl.animate-pulse svg')).not.toBeNull();
  });

  it('words the three empty notes without a name', async () => {
    await setStore({ homeSelectedCategory: NAMELESS, homeActiveTab: 'category', isHomeActive: true });
    await mount();
    await answer(lastCall('get_streams_by_game'), [[], null]);
    expect(text()).toContain('No one is live in this category right now.');

    await click(buttonNamed('Clips'));
    await answer(lastCall('get_clips_by_game'), [[], null]);
    expect(text()).toContain('No clips have been generated for this category yet.');

    await click(buttonNamed('Videos'));
    await answer(lastCall('get_videos_by_game'), [[], null]);
    expect(text()).toContain('No videos exist for this category.');

    // Each of these read the empty name before: "No one is streaming ."
    expect(text()).not.toMatch(/streaming \.| {2}| yet\.\s*yet|for \./);
  });

  it('asks Rust for the name once, and hands the answer to the store', async () => {
    const fill = vi.fn();
    await setStore({ homeSelectedCategory: NAMELESS, homeActiveTab: 'category', isHomeActive: true, fillLinkedCategory: fill });
    await mount();

    expect(records.asks.map((a) => [a.id, a.retry])).toEqual([['509658', false]]);
    await act(async () => {
      records.asks[0].settle({ state: 'found', category: RUST });
    });
    // The answer's own record re-runs the effect, which finds the record and
    // makes no second call to Rust; the store's fill is idempotent.
    expect(records.asks).toHaveLength(1);
    expect(fill).toHaveBeenCalledWith('509658', RUST);
    for (const call of fill.mock.calls) expect(call).toEqual(['509658', RUST]);
  });

  it('fills the name in place: details by the new name only, streams not fetched again', async () => {
    await setStore({ homeSelectedCategory: NAMELESS, homeActiveTab: 'category', isHomeActive: true });
    await mount();
    // The streams land EMPTY and not loading: the re-mount branch's own test
    // for "fetch again", so a fill pass that wrongly runs it fetches twice.
    await answer(lastCall('get_streams_by_game'), [[], null]);
    expect(callsOf('get_category_info')).toEqual([]);

    await open(RUST);

    expect(callsOf('get_streams_by_game')).toHaveLength(1);
    expect(callsOf('get_category_info').map((c) => c.args)).toEqual([{ gameName: 'Rust' }]);
    expect(titleBar()!.textContent).toBe('Rust');
    expect(searchBox()).not.toBeNull();
    expect(buttonNamed('Favorite')).toBeDefined();
    expect(host.querySelector('[aria-label="Share category"]')).not.toBeNull();
    expect(text()).toContain('No one is streaming Rust.');
  });

  it('does not fetch an empty clips or videos list again when the name fills', async () => {
    await setStore({ homeSelectedCategory: NAMELESS, homeActiveTab: 'category', isHomeActive: true });
    await mount();
    await answer(lastCall('get_streams_by_game'), [[], null]);
    await click(buttonNamed('Clips'));
    await answer(lastCall('get_clips_by_game'), [[], null]);
    await click(buttonNamed('Videos'));
    await answer(lastCall('get_videos_by_game'), [[], null]);

    await open(RUST);

    // The fill makes a new category object; an effect keyed on the object would
    // see empty clips and videos and ask again.
    expect(callsOf('get_clips_by_game')).toHaveLength(1);
    expect(callsOf('get_videos_by_game')).toHaveLength(1);
  });

  it('keeps a typed filter while the same id arrives under another spelling of its name', async () => {
    await setStore({ homeSelectedCategory: ALPHA, homeActiveTab: 'category', isHomeActive: true });
    await mount();
    await answer(lastCall('get_streams_by_game'), [[], null]);
    const box = searchBox() as HTMLInputElement;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
      setter.call(box, 'speedrun');
      box.dispatchEvent(new Event('input', { bubbles: true }));
    });
    expect((searchBox() as HTMLInputElement).value).toBe('speedrun');

    // The box clears when the category changes, and the category is its id.
    // Keyed on the name, as it was, this would clear it.
    await open({ ...ALPHA, name: 'Alpha Remastered' });
    expect((searchBox() as HTMLInputElement).value).toBe('speedrun');

    await open(BETA);
    expect((searchBox() as HTMLInputElement).value).toBe('');
  });

  it('after the fill with a long name the chips share the title row, which never wraps', async () => {
    const long = 'A Category Name That Is Far Too Long To Sit Beside Three Chips On One Line';
    await setStore({ homeSelectedCategory: NAMELESS, homeActiveTab: 'category', isHomeActive: true });
    await mount();
    await answer(lastCall('get_streams_by_game'), [[], null]);
    await open({ ...RUST, name: long });
    await answer(lastCall('get_category_info'), info(long, { followersCount: 1234 }));

    const h1 = titleBar()!;
    const row = h1.parentElement!;
    expect(row.className).toContain('flex-nowrap');
    expect(row.className).not.toContain('flex-wrap');
    expect(h1.className).toContain('min-w-0');
    expect(h1.className).toContain('flex-1');
    expect(h1.className).toContain('truncate');
    // The tooltip names the whole title for the part the line cuts off.
    expect(h1.getAttribute('aria-label')).toBe(long);
    expect(row.contains(buttonNamed('Favorite') ?? null)).toBe(true);
    expect(row.contains(host.querySelector('[aria-label="Share category"]'))).toBe(true);
    expect(row.textContent).toContain('Followers');
  });
});

describe('Home with a category whose name never came', () => {
  it('says so in place and offers Try again, keeping the streams below', async () => {
    await setStore({ homeSelectedCategory: NAMELESS, homeActiveTab: 'category', isHomeActive: true });
    await mount();
    await answer(lastCall('get_streams_by_game'), [[stream('s1', 'streamer_one')], null]);
    await act(async () => {
      records.put('509658', { state: 'failed' });
    });

    const line = host.querySelector('p[role="status"]');
    expect(line?.textContent).toBe("Couldn't load this category");
    expect(titleBar()).toBeNull();
    expect(buttonNamed('Try again')).toBeDefined();
    // No skeleton chips left pulsing under a failure; the streams are untouched.
    expect(host.querySelector('.h-5.w-16.animate-pulse')).toBeNull();
    expect(text()).toContain('streamer_one');
  });

  it('Try again retries, stays put while the retry runs, and fills on found', async () => {
    const fill = vi.fn();
    await setStore({ homeSelectedCategory: NAMELESS, homeActiveTab: 'category', isHomeActive: true, fillLinkedCategory: fill });
    records.map.set('509658', { state: 'failed' });
    await mount();
    expect(records.asks).toHaveLength(0);

    await click(buttonNamed('Try again'));

    expect(records.asks.map((a) => [a.id, a.retry])).toEqual([['509658', true]]);
    // The record is `loading` now, and the line and its button are still here.
    expect(host.querySelector('p[role="status"]')?.textContent).toBe("Couldn't load this category");
    const again = buttonNamed('Try again')!;
    expect(again.getAttribute('aria-disabled')).toBe('true');
    expect(titleBar()).toBeNull();
    await click(again);
    expect(records.asks).toHaveLength(1);

    await act(async () => {
      records.asks[0].settle({ state: 'found', category: RUST });
    });
    expect(fill).toHaveBeenCalledWith('509658', RUST);
  });

  it('a missing category has no button, and a record for another id is still the skeleton', async () => {
    await setStore({ homeSelectedCategory: NAMELESS, homeActiveTab: 'category', isHomeActive: true });
    records.map.set('509658', { state: 'missing' });
    await mount();
    expect(host.querySelector('p[role="status"]')?.textContent).toBe("There's no Twitch category with this id.");
    expect(buttonNamed('Try again')).toBeUndefined();

    await act(async () => {
      records.put('509658', null);
      records.put('777', { state: 'failed' });
    });
    expect(host.querySelector('p[role="status"]')).toBeNull();
    expect(titleBar()!.textContent).toBe('Loading category');
  });

  it('the same link again drops the failed record and asks exactly once more', async () => {
    await setStore({ homeSelectedCategory: NAMELESS, homeActiveTab: 'category', isHomeActive: true });
    await mount();
    await act(async () => {
      records.asks[0].settle({ state: 'failed' });
    });
    expect(host.querySelector('p[role="status"]')?.textContent).toBe("Couldn't load this category");
    expect(records.asks).toHaveLength(1);

    // What openCategoryFromLink's forgetFailedLinkedCategory does to the record:
    // it drops it and tells every reader. The fake map has no such hook, so the
    // drop is made by hand beside the real store call.
    await act(async () => {
      records.put('509658', null);
      useAppStore.getState().openCategoryFromLink('509658');
    });

    // The record's state is part of the effect's key: keyed on the id and the
    // name alone, nothing re-runs and the person stays on the failure line.
    expect(records.asks).toHaveLength(2);
    expect(records.asks[1].retry).toBe(false);
    expect(host.querySelector('p[role="status"]')).toBeNull();
    expect(titleBar()!.textContent).toBe('Loading category');
  });
});

describe('Home when the selected category changes', () => {
  it('swaps the header and the streams together on A to B', async () => {
    await setStore({ homeSelectedCategory: ALPHA, homeActiveTab: 'category', isHomeActive: true });
    await mount();
    await answer(lastCall('get_streams_by_game'), [[stream('a1', 'alpha_streamer')], null]);
    await answer(lastCall('get_category_info'), info('Alpha', { id: '1' }));
    expect(titleBar()!.textContent).toBe('Alpha');
    expect(text()).toContain('alpha_streamer');

    await open(BETA);

    // B's header and B's loading state at once: nothing of A is left under it.
    expect(titleBar()!.textContent).toBe('Beta');
    expect(text()).not.toContain('alpha_streamer');
    expect(text()).toContain('Loading streams...');
    expect(callsOf('get_streams_by_game').map((c) => c.args?.gameId)).toEqual(['1', '2']);
    expect(callsOf('get_category_info').map((c) => c.args?.gameName)).toEqual(['Alpha', 'Beta']);

    await answer(lastCall('get_streams_by_game', forId('2')), [[stream('b1', 'beta_streamer')], null]);
    expect(text()).toContain('beta_streamer');
    expect(text()).not.toContain('Loading streams...');
  });

  it('drops a late answer for A once B is showing', async () => {
    await setStore({ homeSelectedCategory: ALPHA, homeActiveTab: 'category', isHomeActive: true });
    await mount();
    const lateStreams = lastCall('get_streams_by_game', forId('1'));
    const lateInfo = lastCall('get_category_info', forName('Alpha'));
    await open(BETA);

    await answer(lateStreams, [[stream('a1', 'alpha_streamer')], null]);
    await answer(lateInfo, info('Alpha', { id: '1', displayName: 'Alpha Late Header' }));

    // Neither A's streams nor A's header land over B, and B's spinner is still B's.
    expect(text()).not.toContain('alpha_streamer');
    expect(text()).not.toContain('Alpha Late Header');
    expect(titleBar()!.textContent).toBe('Beta');
    expect(text()).toContain('Loading streams...');

    await answer(lastCall('get_streams_by_game', forId('2')), [[stream('b1', 'beta_streamer')], null]);
    expect(text()).toContain('beta_streamer');
    expect(text()).not.toContain('alpha_streamer');
  });

  it('on A to B to A the first A answer neither lands nor clears the newer spinner', async () => {
    await setStore({ homeSelectedCategory: ALPHA, homeActiveTab: 'category', isHomeActive: true });
    await mount();
    const firstA = lastCall('get_streams_by_game', forId('1'));
    await open(BETA);
    await open(ALPHA);
    expect(callsOf('get_streams_by_game').map((c) => c.args?.gameId)).toEqual(['1', '2', '1']);
    const secondA = lastCall('get_streams_by_game', forId('1'));
    expect(secondA).not.toBe(firstA);

    // Same key, older token: the key alone cannot tell these two loads apart.
    await answer(firstA, [[stream('old', 'first_try_streamer')], null]);
    expect(text()).not.toContain('first_try_streamer');
    expect(text()).toContain('Loading streams...');

    await answer(secondA, [[stream('new', 'second_try_streamer')], null]);
    expect(text()).toContain('second_try_streamer');
    expect(text()).not.toContain('Loading streams...');
  });

  it('a failed streams answer ends the spinner for the current category, and a stale failure does nothing', async () => {
    await setStore({ homeSelectedCategory: ALPHA, homeActiveTab: 'category', isHomeActive: true });
    await mount();
    const staleA = lastCall('get_streams_by_game', forId('1'));
    await open(BETA);

    // A's failure arrives after B is up: it must not clear B's spinner or empty B's list.
    await refuse(staleA, new Error('late failure'));
    expect(text()).toContain('Loading streams...');

    await refuse(lastCall('get_streams_by_game', forId('2')), new Error('down'));
    expect(text()).not.toContain('Loading streams...');
    expect(text()).toContain('No Live Streams');
  });

  it('a fresh mount adopts the selection and its answer clears the spinner', async () => {
    await setStore({ homeSelectedCategory: ALPHA, homeActiveTab: 'category', isHomeActive: true });
    await mount();
    expect(text()).toContain('Loading streams...');
    await answer(lastCall('get_streams_by_game'), [[stream('a1', 'alpha_streamer')], null]);
    // A loader that read the ref before the pass wrote it, or a ref written at
    // the end of the pass, drops this answer and the spinner stays.
    expect(text()).toContain('alpha_streamer');
    expect(text()).not.toContain('Loading streams...');
    expect(callsOf('get_streams_by_game')).toHaveLength(1);
  });

  it('a category opened from a stream card resets and drops the old category\'s late answer', async () => {
    const card: TwitchStream = { ...stream('r1', 'recommended_one'), game_id: '77', game_name: 'Gamma' };
    await setStore({
      homeSelectedCategory: ALPHA,
      homeActiveTab: 'category',
      isHomeActive: true,
      activePlatform: 'twitch',
      recommendedStreams: [card],
    });
    await mount();
    const lateA = lastCall('get_streams_by_game', forId('1'));
    await setStore({ homeActiveTab: 'recommended' });
    await click(buttonNamed('Gamma'));

    expect(callsOf('get_streams_by_game').map((c) => c.args?.gameId)).toEqual(['1', '77']);
    expect(callsOf('get_category_info').map((c) => c.args?.gameName)).toEqual(['Alpha', 'Gamma']);
    await answer(lateA, [[stream('a1', 'alpha_streamer')], null]);
    expect(text()).not.toContain('alpha_streamer');
    expect(text()).toContain('Loading streams...');

    await answer(lastCall('get_streams_by_game', forId('77')), [[stream('g1', 'gamma_streamer')], null]);
    expect(text()).toContain('gamma_streamer');
    expect(text()).not.toContain('Loading streams...');
  });

  it('a name that arrives while the page was away is a return, so the streams load again', async () => {
    await setStore({ homeSelectedCategory: NAMELESS, homeActiveTab: 'category', isHomeActive: true });
    await mount();
    await answer(lastCall('get_streams_by_game'), [[stream('s1', 'streamer_one')], null]);

    // Away: Home drops the category's streams when it is not the category page.
    await setStore({ homeActiveTab: 'following' });
    await setStore({ homeSelectedCategory: RUST });
    await setStore({ homeActiveTab: 'category' });

    // A fill-only pass would fetch the details and leave the streams empty.
    expect(callsOf('get_streams_by_game')).toHaveLength(2);
    expect(callsOf('get_category_info').map((c) => c.args?.gameName)).toEqual(['Rust']);
  });
});

describe('the Share button', () => {
  const clipboard = (impl: (text: string) => Promise<void>) => {
    const writeText = vi.fn(function (this: unknown, value: string) {
      // A bare `navigator.clipboard.writeText` has no `this`: browsers throw.
      if (this !== navigator.clipboard) throw new TypeError('Illegal invocation');
      return impl(value);
    });
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    return writeText;
  };
  const share = () => host.querySelector('[aria-label="Share category"]') as HTMLButtonElement;
  const status = () => Array.from(host.querySelectorAll('[role="status"]')).map((e) => e.textContent).join('|');

  it('copies the link for the id and name only and says "Link copied!" for a moment', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const writeText = clipboard(async () => {});
    await setStore({ homeSelectedCategory: { ...RUST, box_art_url: 'https://art/x.jpg' }, homeActiveTab: 'category', isHomeActive: true });
    await mount();
    expect(share().getAttribute('title')).toBeNull();

    await click(share());
    const ask = lastCall('category_share_link');
    // Two keys: a category passed whole must not send its art URL.
    expect(ask.args).toEqual({ id: '509658', name: 'Rust' });
    expect(status()).toBe('');
    await answer(ask, 'https://streamnook.app/c/509658/rust');
    await act(async () => {
      await Promise.resolve();
    });

    expect(writeText).toHaveBeenCalledWith('https://streamnook.app/c/509658/rust');
    expect(status()).toBe('Link copied!');

    await act(async () => {
      vi.advanceTimersByTime(1100);
    });
    expect(status()).toBe('');
  });

  it('says nothing and copies nothing when the link is refused, and nothing rejects', async () => {
    const writeText = clipboard(async () => {});
    await setStore({ homeSelectedCategory: RUST, homeActiveTab: 'category', isHomeActive: true });
    await mount();

    await click(share());
    await refuse(lastCall('category_share_link'), new Error('bad id'));
    await act(async () => {
      await Promise.resolve();
    });

    expect(writeText).not.toHaveBeenCalled();
    expect(status()).toBe('');
  });

  it('says nothing when the clipboard refuses', async () => {
    clipboard(async () => {
      throw new Error('denied');
    });
    await setStore({ homeSelectedCategory: RUST, homeActiveTab: 'category', isHomeActive: true });
    await mount();

    await click(share());
    await answer(lastCall('category_share_link'), 'https://streamnook.app/c/509658/rust');
    await act(async () => {
      await Promise.resolve();
    });

    expect(status()).toBe('');
  });

  it('is not on another platform\'s category drill-down', async () => {
    await setStore({
      activePlatform: 'kick',
      homeActiveTab: 'browse',
      isHomeActive: true,
      homeProviderCategory: {
        provider: 'kick',
        id: '5',
        name: 'Just Chatting',
        thumbnail: '',
        viewer_count: 0,
        channel_count: 0,
      },
    });
    await mount();
    expect(host.querySelector('[aria-label="Share category"]')).toBeNull();
  });
});

describe('the automatic tab choice', () => {
  it('goes through autoSelectHomeTab, never the tab click setter', async () => {
    const auto = vi.fn();
    const setTab = vi.fn();
    await setStore({
      homeActiveTab: 'following',
      homeSelectedCategory: null,
      isAuthenticated: false,
      isHomeActive: true,
      autoSelectHomeTab: auto,
      setHomeActiveTab: setTab,
    });
    await mount();

    // The boot watch treats a tab click as the person acting; the automatic
    // choice has to arrive through the one door that it ignores.
    expect(auto).toHaveBeenCalledWith('recommended');
    expect(setTab).not.toHaveBeenCalled();
  });
});
