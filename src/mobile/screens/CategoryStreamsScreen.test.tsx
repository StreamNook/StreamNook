// @vitest-environment happy-dom
// The phone's category screen as a link opens it: nameless at first, named in
// place, its failure and not-found lines in the header, and loads that never
// let an older answer write over a newer one.
import type { ReactNode } from 'react';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { MotionGlobalConfig } from 'framer-motion';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { TwitchCategory, TwitchStream } from '../../types';

MotionGlobalConfig.skipAnimations = true;
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

// Every streams call is held until the test answers it, so the test decides
// the order answers land in.
const rust = vi.hoisted(() => {
  type Call = {
    cmd: string;
    args: Record<string, unknown> | undefined;
    resolve: (value: unknown) => void;
    reject: (reason: unknown) => void;
  };
  const calls: Call[] = [];
  const HELD = new Set(['get_streams_by_game', 'get_streams_by_game_with_tags']);
  const invoke = (cmd: string, args?: Record<string, unknown>) => {
    if (cmd === 'category_share_link') return Promise.resolve('https://streamnook.app/c/509658/just-chatting');
    if (!HELD.has(cmd)) return Promise.resolve(null);
    return new Promise((resolve, reject) => {
      calls.push({ cmd, args, resolve, reject });
    });
  };
  return { calls, invoke };
});

// The linked-category records, driven by hand: this file tests what the
// screen does with a record, not how the record is made.
const records = vi.hoisted(() => {
  type Rec = { state: 'loading' | 'found' | 'failed' | 'missing'; category?: TwitchCategory };
  type Ask = { id: string; retry: boolean; settle: (rec: Rec) => void };
  const map = new Map<string, Rec>();
  const listeners = new Set<() => void>();
  const asks: Ask[] = [];
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

const io = vi.hoisted(() => ({
  callbacks: [] as Array<(entries: Array<{ isIntersecting: boolean }>) => void>,
}));

const share = vi.hoisted(() => ({ text: vi.fn((_text: string, _subject?: string) => true) }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: rust.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('../../stores/AppStore', () => {
  const state = {
    startStream: vi.fn(),
    activeHypeTrainChannels: new Map(),
    watchStreaks: {},
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
vi.mock('../../utils/linkedCategory', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../utils/linkedCategory')>();
  const { useSyncExternalStore } = await import('react');
  return {
    ...actual,
    resolveLinkedCategory: records.resolve,
    getLinkedCategory: (id: string) => records.map.get(id),
    useLinkedCategory: (id: string | null) =>
      useSyncExternalStore(
        (listener: () => void) => {
          records.listeners.add(listener);
          return () => records.listeners.delete(listener);
        },
        () => (id ? records.map.get(id) : undefined),
        () => (id ? records.map.get(id) : undefined),
      ),
  };
});
vi.mock('../nativeBridge', () => ({ shareText: share.text }));
vi.mock('../../components/ui/Tooltip', () => ({ Tooltip: ({ children }: { children: ReactNode }) => children }));
vi.mock('../ui/DrillInScreen', () => ({
  DrillInScreen: ({ open, children }: { open: boolean; children: ReactNode }) => (open ? <div>{children}</div> : null),
}));
vi.mock('../useHypeTrains', () => ({ useHypeTrains: () => {} }));
vi.mock('../ui/MobileStreamCard', () => ({
  MobileStreamCard: ({
    stream,
    tagFilter,
  }: {
    stream: { user_login: string };
    tagFilter?: { onToggle: (tag: string) => void };
  }) => (
    <div data-testid="card" data-login={stream.user_login}>
      <button type="button" data-testid="card-tag" onClick={() => tagFilter?.onToggle('English')}>
        English
      </button>
    </div>
  ),
}));
vi.mock('../ui/AdaptiveGrid', () => ({ AdaptiveGrid: ({ children }: { children: ReactNode }) => <div>{children}</div> }));
vi.mock('../ui/SettleIn', () => ({
  SettleIn: ({ children }: { children: ReactNode }) => <>{children}</>,
  useSettleIn: () => true,
}));
vi.mock('../ui/PullToRefresh', () => ({
  PullToRefresh: ({ onRefresh, children }: { onRefresh: () => unknown; children: ReactNode }) => (
    <div>
      <button type="button" data-testid="pull" onClick={() => void onRefresh()} />
      {children}
    </div>
  ),
}));
vi.mock('../ui/SkeletonCards', () => ({ SkeletonCards: () => <div data-testid="skeleton" /> }));
vi.mock('../ui/CategoryTagSheet', () => ({
  CategoryTagSheet: ({ open }: { open: boolean }) => (open ? <div data-testid="tag-sheet" /> : null),
}));
vi.mock('../../hooks/useCategoryTagSuggestions', () => ({
  useCategoryTagSuggestions: ({ gameName }: { gameName: string }) => (gameName ? { labels: ['English'] } : null),
}));
vi.mock('../followRefresh', () => ({ bumpPreviewStamp: () => {} }));

class FakeObserver {
  constructor(callback: (entries: Array<{ isIntersecting: boolean }>) => void) {
    io.callbacks.push(callback);
  }
  observe() {}
  unobserve() {}
  disconnect() {}
  takeRecords() {
    return [];
  }
}
(globalThis as { IntersectionObserver?: unknown }).IntersectionObserver = FakeObserver;

import { CategoryStreamsScreen } from './CategoryStreamsScreen';
import { useMobileNavStore } from '../navStore';

const NAMELESS: TwitchCategory = { id: '509658', name: '', box_art_url: '' };
const NAMED: TwitchCategory = { id: '509658', name: 'Just Chatting', box_art_url: 'art' };

const stream = (login: string) =>
  ({
    id: login,
    user_id: `u${login}`,
    user_login: login,
    user_name: login,
    title: '',
    viewer_count: 1,
    game_id: '509658',
    game_name: 'Just Chatting',
    thumbnail_url: '',
    started_at: '2026-01-01T00:00:00Z',
  }) as TwitchStream;

let root: Root;
let host: HTMLDivElement;

const flush = () =>
  act(async () => {
    await new Promise((done) => setTimeout(done, 0));
  });
const calls = (cmd = 'get_streams_by_game') => rust.calls.filter((c) => c.cmd === cmd);
const shown = () => [...document.querySelectorAll('[data-testid="card"]')].map((el) => el.getAttribute('data-login'));
const text = () => document.body.textContent ?? '';
const button = (label: string) => document.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`);
const buttonByText = (words: string) =>
  [...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent === words);

async function answer(cmd: string, index: number, logins: string[], cursor: string | null) {
  await act(async () => {
    calls(cmd)[index].resolve([logins.map(stream), cursor]);
  });
  await flush();
}

async function fail(cmd: string, index: number) {
  await act(async () => {
    calls(cmd)[index].reject(new Error('offline'));
  });
  await flush();
}

async function click(el: HTMLElement | null | undefined) {
  if (!el) throw new Error('nothing to click');
  await act(async () => {
    el.click();
  });
  await flush();
}

async function nearEnd(callback = io.callbacks[io.callbacks.length - 1]) {
  await act(async () => {
    callback?.([{ isIntersecting: true }]);
  });
  await flush();
}

async function mount(category: TwitchCategory) {
  await act(async () => {
    useMobileNavStore.setState({ browseCategory: category });
    root.render(<CategoryStreamsScreen />);
  });
  await flush();
}

beforeEach(() => {
  rust.calls.length = 0;
  records.map.clear();
  records.asks.length = 0;
  records.inFlight.clear();
  io.callbacks.length = 0;
  share.text.mockClear();
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  useMobileNavStore.setState({ browseCategory: null });
});

describe('a link opens the category nameless', () => {
  // Fails if the header waits for the name, renders an empty h1, or the Tags chip opens a search with no category.
  it('shows the loading title, a live Back, a dimmed Tags chip and no Share, and loads by id', async () => {
    await mount(NAMELESS);
    expect(document.querySelector('h1')).toBeNull();
    const bar = document.querySelector('.animate-pulse');
    expect(bar?.getAttribute('aria-hidden')).toBe('true');
    expect(text()).toContain('Loading category');
    expect(button('Back to Browse')?.disabled).toBe(false);
    const tags = button('Search tags');
    expect(tags?.disabled).toBe(true);
    expect(tags?.getAttribute('aria-disabled')).toBe('true');
    expect(button('Share category')).toBeNull();
    expect(calls()).toHaveLength(1);
    expect(calls()[0].args).toMatchObject({ gameId: '509658', cursor: null });
    expect(records.asks.map((a) => a.id)).toEqual(['509658']);
  });

  // Fails if the fill refetches the streams (load keyed on the object) or the rail row mounts late.
  it('fills the name in place: title, tags and Share appear, the rail keeps its node, no second load', async () => {
    await mount(NAMELESS);
    const rail = button('Search tags')?.parentElement;
    const railIndex = rail?.parentElement ? [...rail.parentElement.children].indexOf(rail) : -1;
    await act(async () => {
      records.asks[0].settle({ state: 'found', category: NAMED });
    });
    await flush();
    expect(document.querySelector('h1')?.textContent).toBe('Just Chatting');
    expect(button('Share category')).not.toBeNull();
    expect(button('Search tags')?.disabled).toBe(false);
    const railAfter = button('Search tags')?.parentElement;
    expect(railAfter).toBe(rail);
    expect(railAfter?.parentElement ? [...railAfter.parentElement.children].indexOf(railAfter) : -2).toBe(railIndex);
    expect(calls()).toHaveLength(1);
  });

  it('names itself from a found record with no ask', async () => {
    records.put('509658', { state: 'found', category: NAMED });
    await mount(NAMELESS);
    expect(document.querySelector('h1')?.textContent).toBe('Just Chatting');
    expect(records.asks).toHaveLength(0);
  });
});

describe('the header when the name did not load', () => {
  // Fails if Try again only asks for the name and leaves a failed streams list in place.
  it('says it failed, and Try again asks again and runs the streams load once more', async () => {
    records.put('509658', { state: 'failed' });
    await mount(NAMELESS);
    expect(text()).toContain("Couldn't load this category");
    await fail('get_streams_by_game', 0);
    expect(text()).toContain("Couldn't load streams");
    expect(text()).not.toContain('No live streams in this category.');
    await click(buttonByText('Try again'));
    expect(records.asks.at(-1)).toMatchObject({ id: '509658', retry: true });
    expect(calls()).toHaveLength(2);
    expect(document.querySelector('[data-testid="skeleton"]')).not.toBeNull();
    await answer('get_streams_by_game', 1, ['a'], null);
    expect(shown()).toEqual(['a']);
  });

  // Fails if a missing category shows two messages or offers a retry that cannot help.
  it('says once that there is no such category, with no Try again and the Tags chip hidden in place', async () => {
    records.put('509658', { state: 'missing' });
    await mount(NAMELESS);
    expect(text()).toContain("There's no Twitch category with this id.");
    expect(buttonByText('Try again')).toBeUndefined();
    expect(button('Search tags')?.className).toContain('invisible');
    await answer('get_streams_by_game', 0, [], null);
    expect(text()).not.toContain('No live streams in this category.');
    expect(text()).not.toContain("Couldn't load streams");
  });
});

describe('a failed load still has a way out', () => {
  // Fails if the screen waits for the name lookup alone, leaving the card chips dead when it fails.
  it('names itself from its own streams when the name lookup failed, and a chip then filters', async () => {
    records.put('509658', { state: 'failed' });
    await mount(NAMELESS);
    await answer('get_streams_by_game', 0, ['a'], null);
    expect(document.querySelector('h1')?.textContent).toBe('Just Chatting');
    await click(document.querySelector<HTMLElement>('[data-testid="card-tag"]'));
    expect(calls('get_streams_by_game_with_tags')).toHaveLength(1);
  });

  // Fails if a failed streams load under a known name hides the pull hint.
  it('keeps the pull hint under a failed streams load', async () => {
    await mount(NAMED);
    await fail('get_streams_by_game', 0);
    expect(text()).toContain("Couldn't load streams");
    expect(text()).toContain('Pull down to refresh.');
  });

  // Fails if another link to the category retries only the name and leaves the failed list.
  it('another link to this category loads the failed streams again', async () => {
    await mount(NAMED);
    await fail('get_streams_by_game', 0);
    await act(async () => {
      useMobileNavStore.getState().openCategoryLink('509658');
    });
    await flush();
    expect(calls()).toHaveLength(2);
    await answer('get_streams_by_game', 1, ['a'], null);
    expect(shown()).toEqual(['a']);
  });

  // Fails if any link bump reloads a list that loaded fine.
  it('another link leaves a list that loaded fine alone', async () => {
    await mount(NAMED);
    await answer('get_streams_by_game', 0, ['a'], null);
    await act(async () => {
      useMobileNavStore.getState().openCategoryLink('509658');
    });
    await flush();
    expect(calls()).toHaveLength(1);
  });
});

describe('tags wait for the name', () => {
  // Fails if a chip on a card filters by tag while the screen has no name to filter in.
  it('a card chip does nothing while nameless and filters once named', async () => {
    await mount(NAMELESS);
    // Streams of another category name nothing, so the screen stays nameless.
    await act(async () => {
      calls()[0].resolve([[{ ...stream('a'), game_id: '1', game_name: 'Other' }], null]);
    });
    await flush();
    await click(document.querySelector<HTMLElement>('[data-testid="card-tag"]'));
    expect(calls('get_streams_by_game_with_tags')).toHaveLength(0);
    await act(async () => {
      useMobileNavStore.getState().fillBrowseCategory('509658', NAMED);
    });
    await flush();
    await click(document.querySelector<HTMLElement>('[data-testid="card-tag"]'));
    expect(calls('get_streams_by_game_with_tags')).toHaveLength(1);
  });

  // Fails if the dimmed Tags chip still opens the sheet.
  it('the Tags chip opens no sheet while nameless, and opens it once named', async () => {
    await mount(NAMELESS);
    await click(button('Search tags'));
    expect(document.querySelector('[data-testid="tag-sheet"]')).toBeNull();
    await act(async () => {
      useMobileNavStore.getState().fillBrowseCategory('509658', NAMED);
    });
    await flush();
    await click(button('Search tags'));
    expect(document.querySelector('[data-testid="tag-sheet"]')).not.toBeNull();
  });
});

describe('loads never let an older answer win', () => {
  // Fails without the load number: the older load's late failure wipes the newer list.
  it('a late failure of an older load leaves the newer list', async () => {
    await mount(NAMED);
    await click(document.querySelector<HTMLElement>('[data-testid="pull"]'));
    expect(calls()).toHaveLength(2);
    await answer('get_streams_by_game', 1, ['new'], null);
    await fail('get_streams_by_game', 0);
    expect(shown()).toEqual(['new']);
    expect(text()).not.toContain("Couldn't load streams");
  });

  // Fails without the newest-load check after the await in load.
  it('a late answer of an older load writes nothing, and paging goes on from the newer cursor', async () => {
    await mount(NAMED);
    await click(document.querySelector<HTMLElement>('[data-testid="pull"]'));
    await answer('get_streams_by_game', 1, ['new'], 'c2');
    await answer('get_streams_by_game', 0, ['old'], 'c1');
    expect(shown()).toEqual(['new']);
    await nearEnd();
    expect(calls()[calls().length - 1].args).toMatchObject({ cursor: 'c2' });
  });

  // Fails when an older load's finally clears loading while the newer load runs.
  it('an older load ending first keeps the skeleton while the newer load runs', async () => {
    await mount(NAMED);
    await click(document.querySelector<HTMLElement>('[data-testid="pull"]'));
    await fail('get_streams_by_game', 0);
    expect(document.querySelector('[data-testid="skeleton"]')).not.toBeNull();
    expect(text()).not.toContain("Couldn't load streams");
  });

  // Fails if loadMore's answer lands after a refresh that started later.
  it('a page asked before a refresh is not appended after it', async () => {
    await mount(NAMED);
    await answer('get_streams_by_game', 0, ['a'], 'c1');
    await nearEnd();
    expect(calls()[1].args).toMatchObject({ cursor: 'c1' });
    await click(document.querySelector<HTMLElement>('[data-testid="pull"]'));
    await answer('get_streams_by_game', 2, ['b'], 'c2');
    await answer('get_streams_by_game', 1, ['z'], 'c9');
    expect(shown()).toEqual(['b']);
    await nearEnd();
    expect(calls()[calls().length - 1].args).toMatchObject({ cursor: 'c2' });
  });

  // Fails if loadMore asks with the old cursor while a refresh is still pending.
  it('no page is asked for while a refresh is pending', async () => {
    await mount(NAMED);
    await answer('get_streams_by_game', 0, ['a'], 'c1');
    await click(document.querySelector<HTMLElement>('[data-testid="pull"]'));
    await nearEnd();
    expect(calls().filter((c) => c.args?.cursor === 'c1')).toHaveLength(0);
    await answer('get_streams_by_game', 1, ['b'], 'c2');
    expect(shown()).toEqual(['b']);
  });

  // Fails if a stale observer callback can still ask with the cursor it saw.
  it("an old observer callback never asks with the old cursor", async () => {
    await mount(NAMED);
    await answer('get_streams_by_game', 0, ['a'], 'c1');
    const old = io.callbacks[io.callbacks.length - 1];
    await click(document.querySelector<HTMLElement>('[data-testid="pull"]'));
    await answer('get_streams_by_game', 1, ['b'], 'c2');
    await nearEnd(old);
    expect(calls().filter((c) => c.args?.cursor === 'c1')).toHaveLength(0);
  });

  // Fails if the page check sits before the tag branch: tag pages would stop after the first.
  it('tag pages still page after a failed plain load', async () => {
    await mount(NAMED);
    await fail('get_streams_by_game', 0);
    const chip = [...document.querySelectorAll<HTMLButtonElement>('button[aria-pressed]')].find(
      (b) => b.textContent === 'English',
    );
    await click(chip);
    expect(calls('get_streams_by_game_with_tags')).toHaveLength(1);
    await answer('get_streams_by_game_with_tags', 0, ['t'], 't1');
    await nearEnd();
    expect(calls('get_streams_by_game_with_tags')).toHaveLength(2);
    expect(calls('get_streams_by_game_with_tags')[1].args).toMatchObject({ cursor: 't1' });
  });
});

describe('Share', () => {
  it('hands the category link to the system share sheet', async () => {
    await mount(NAMED);
    await click(button('Share category'));
    expect(share.text).toHaveBeenCalledWith(
      'Just Chatting on StreamNook: https://streamnook.app/c/509658/just-chatting',
      'Just Chatting',
    );
  });

  // Fails if the handler grows its own catch or label, or the button gains a title= hint.
  it('goes through the one catch in copyCategoryShareLink, with no label and no title', () => {
    const source = readFileSync(resolve(__dirname, 'CategoryStreamsScreen.tsx'), 'utf8').replace(/\s+/g, ' ');
    expect(source).toContain('copyCategoryShareLink(category, (link) => shareText(');
    expect(source).toContain("? Promise.resolve() : Promise.reject(new Error('no share'))");
    const handler = source.slice(source.indexOf('const share = () =>'), source.indexOf('const empty ='));
    expect(handler).not.toContain('.catch(');
    expect(source).not.toMatch(/\btitle=/);
  });
});
