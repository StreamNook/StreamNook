import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';

const h = vi.hoisted(() => ({ invokeSpy: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invokeSpy }));

import {
  categoryAnswerOutcome,
  forgetFailedLinkedCategory,
  getLinkedCategory,
  linkedCategorySizesForTest,
  onLateLinkedCategory,
  resetLinkedCategories,
  resolveLinkedCategory,
  subscribeLinkedCategories,
  useLinkedCategory,
  type LinkedCategory,
} from './linkedCategory';

const ONE_HOUR = 60 * 60 * 1000;

const found = (id: string, name: string) => ({
  state: 'found',
  id,
  name,
  box_art_url: `art-${id}`,
  viewer_count: null,
});

function deferred<T = unknown>() {
  let resolveIt: (value: T) => void = () => {};
  let rejectIt: (error: unknown) => void = () => {};
  const promise = new Promise<T>((res, rej) => {
    resolveIt = res;
    rejectIt = rej;
  });
  return { promise, resolve: resolveIt, reject: rejectIt };
}

// Lets promise callbacks run without moving the clock.
const settle = () => vi.advanceTimersByTimeAsync(0);

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(1_000_000);
  h.invokeSpy.mockReset();
  resetLinkedCategories();
});

afterEach(() => {
  resetLinkedCategories();
  vi.useRealTimers();
});

describe('resolveLinkedCategory', () => {
  // Catches a resolve that asks again for an id it already knows.
  it('stores found and makes no second call', async () => {
    h.invokeSpy.mockResolvedValue(found('509658', 'Just Chatting'));
    const record = await resolveLinkedCategory('509658');
    expect(record).toEqual({
      state: 'found',
      category: { id: '509658', name: 'Just Chatting', box_art_url: 'art-509658' },
      at: 1_000_000,
    });
    expect(h.invokeSpy).toHaveBeenCalledWith('resolve_category_link', { id: '509658' });
    await resolveLinkedCategory('509658');
    expect(h.invokeSpy).toHaveBeenCalledTimes(1);
  });

  // Catches two screens for one nameless id making two lookups.
  it('two callers of one id share one call and one promise', async () => {
    const d = deferred();
    h.invokeSpy.mockReturnValue(d.promise);
    const first = resolveLinkedCategory('1');
    const second = resolveLinkedCategory('1');
    expect(second).toBe(first);
    expect(getLinkedCategory('1')?.state).toBe('loading');
    d.resolve(found('1', 'Rust'));
    await expect(first).resolves.toMatchObject({ state: 'found' });
    expect(h.invokeSpy).toHaveBeenCalledTimes(1);
  });

  // Catches a rejected call or an unreadable answer escaping as a rejection
  // instead of a failed record.
  it('a rejected call, an unreadable payload and an unreachable answer store failed', async () => {
    h.invokeSpy.mockRejectedValueOnce(new Error('ipc'));
    await expect(resolveLinkedCategory('1')).resolves.toEqual({ state: 'failed' });
    h.invokeSpy.mockResolvedValueOnce({ state: 'weird' });
    await expect(resolveLinkedCategory('2')).resolves.toEqual({ state: 'failed' });
    h.invokeSpy.mockResolvedValueOnce(null);
    await expect(resolveLinkedCategory('3')).resolves.toEqual({ state: 'failed' });
    h.invokeSpy.mockResolvedValueOnce({ state: 'unreachable' });
    await expect(resolveLinkedCategory('4')).resolves.toEqual({ state: 'failed' });
  });

  // Catches a failed record that asks again by itself, or a retry that does not.
  it('returns failed as it is, and asks again only with retry', async () => {
    h.invokeSpy.mockRejectedValueOnce(new Error('ipc'));
    await resolveLinkedCategory('1');
    await resolveLinkedCategory('1');
    expect(h.invokeSpy).toHaveBeenCalledTimes(1);
    h.invokeSpy.mockResolvedValueOnce(found('1', 'Rust'));
    const record = await resolveLinkedCategory('1', { retry: true });
    expect(record.state).toBe('found');
    expect(h.invokeSpy).toHaveBeenCalledTimes(2);
  });

  // Catches a missing category being retried: a retry cannot change it.
  it('never asks again for missing, retry or not', async () => {
    h.invokeSpy.mockResolvedValueOnce({ state: 'missing' });
    await expect(resolveLinkedCategory('1')).resolves.toEqual({ state: 'missing' });
    await resolveLinkedCategory('1', { retry: true });
    await resolveLinkedCategory('1');
    expect(h.invokeSpy).toHaveBeenCalledTimes(1);
  });

  // Catches a found record treated as a failure for a retry.
  it('retry on a found record changes nothing', async () => {
    h.invokeSpy.mockResolvedValue(found('1', 'Rust'));
    await resolveLinkedCategory('1');
    await resolveLinkedCategory('1', { retry: true });
    expect(h.invokeSpy).toHaveBeenCalledTimes(1);
  });
});

describe('forgetFailedLinkedCategory', () => {
  // Catches a forget that keeps a failed or missing record (the link would not
  // be a fresh attempt) or drops a found one.
  it('drops failed and missing, never found', async () => {
    h.invokeSpy.mockRejectedValueOnce(new Error('x'));
    await resolveLinkedCategory('f');
    h.invokeSpy.mockResolvedValueOnce({ state: 'missing' });
    await resolveLinkedCategory('m');
    h.invokeSpy.mockResolvedValueOnce(found('k', 'Kept'));
    await resolveLinkedCategory('k');
    forgetFailedLinkedCategory('f');
    forgetFailedLinkedCategory('m');
    forgetFailedLinkedCategory('k');
    expect(getLinkedCategory('f')).toBeUndefined();
    expect(getLinkedCategory('m')).toBeUndefined();
    expect(getLinkedCategory('k')?.state).toBe('found');
  });

  // Catches a silent drop: every reader's effect would stay asleep.
  it('notifies when it drops something and only then', async () => {
    h.invokeSpy.mockRejectedValueOnce(new Error('x'));
    await resolveLinkedCategory('f');
    const listener = vi.fn();
    const unsubscribe = subscribeLinkedCategories(listener);
    forgetFailedLinkedCategory('f');
    expect(listener).toHaveBeenCalledTimes(1);
    forgetFailedLinkedCategory('f');
    forgetFailedLinkedCategory('never');
    expect(listener).toHaveBeenCalledTimes(1);
    unsubscribe();
  });

  // Catches a forget that cancels a resolve in flight: it must change nothing,
  // issue no token and notify nobody.
  it('on a loading record changes nothing', async () => {
    const d = deferred();
    h.invokeSpy.mockReturnValue(d.promise);
    void resolveLinkedCategory('1');
    const before = linkedCategorySizesForTest();
    const listener = vi.fn();
    subscribeLinkedCategories(listener);
    forgetFailedLinkedCategory('1');
    expect(listener).not.toHaveBeenCalled();
    expect(linkedCategorySizesForTest()).toEqual(before);
    expect(getLinkedCategory('1')?.state).toBe('loading');
  });
});

describe('late answers', () => {
  // Catches a late found being dropped: the first call is the only answer, so
  // it names the category in place and reaches the store's copies.
  it('a hung call fails at 6 s and its found at 8 s fills the record and the listeners', async () => {
    const d = deferred();
    h.invokeSpy.mockReturnValue(d.promise);
    const late = vi.fn();
    onLateLinkedCategory(late);
    const result = resolveLinkedCategory('1');
    await vi.advanceTimersByTimeAsync(5999);
    expect(getLinkedCategory('1')?.state).toBe('loading');
    await vi.advanceTimersByTimeAsync(1);
    await expect(result).resolves.toEqual({ state: 'failed' });
    expect(getLinkedCategory('1')).toEqual({ state: 'failed' });
    await vi.advanceTimersByTimeAsync(2000);
    d.resolve(found('1', 'Stale Name'));
    await settle();
    expect(getLinkedCategory('1')).toMatchObject({
      state: 'found',
      category: { id: '1', name: 'Stale Name' },
    });
    expect(late).toHaveBeenCalledTimes(1);
    expect(late).toHaveBeenCalledWith('1', { id: '1', name: 'Stale Name', box_art_url: 'art-1' });
  });

  // Catches a late missing that is dropped, or a late unreachable that
  // replaces the failed record.
  it('a late missing replaces failed and a late unreachable leaves it', async () => {
    const missing = deferred();
    const unreachable = deferred();
    h.invokeSpy.mockReturnValueOnce(missing.promise).mockReturnValueOnce(unreachable.promise);
    const late = vi.fn();
    onLateLinkedCategory(late);
    void resolveLinkedCategory('m');
    void resolveLinkedCategory('u');
    await vi.advanceTimersByTimeAsync(6000);
    missing.resolve({ state: 'missing' });
    unreachable.resolve({ state: 'unreachable' });
    await settle();
    expect(getLinkedCategory('m')).toEqual({ state: 'missing' });
    expect(getLinkedCategory('u')).toEqual({ state: 'failed' });
    expect(late).not.toHaveBeenCalled();
  });

  // Catches a retry that joins the dead promise, or an old answer that
  // overwrites the retry's.
  it('a retry at 7 s makes a new call and the first call 8 s answer writes nothing', async () => {
    const first = deferred();
    const second = deferred();
    h.invokeSpy.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    void resolveLinkedCategory('1');
    await vi.advanceTimersByTimeAsync(7000);
    const retry = resolveLinkedCategory('1', { retry: true });
    expect(h.invokeSpy).toHaveBeenCalledTimes(2);
    second.resolve(found('1', 'Fresh Name'));
    await expect(retry).resolves.toMatchObject({ category: { name: 'Fresh Name' } });
    await vi.advanceTimersByTimeAsync(1000);
    first.resolve(found('1', 'Stale Name'));
    await settle();
    expect(getLinkedCategory('1')).toMatchObject({ category: { name: 'Fresh Name' } });
  });

  // Catches an old answer that lands over a retry still in flight.
  it('the first call answer while the retry is still loading writes nothing', async () => {
    const first = deferred();
    const second = deferred();
    h.invokeSpy.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    void resolveLinkedCategory('1');
    await vi.advanceTimersByTimeAsync(7000);
    void resolveLinkedCategory('1', { retry: true });
    first.resolve(found('1', 'Stale Name'));
    await settle();
    expect(getLinkedCategory('1')).toEqual({ state: 'loading' });
    second.resolve(found('1', 'Fresh Name'));
    await settle();
    expect(getLinkedCategory('1')).toMatchObject({ category: { name: 'Fresh Name' } });
  });

  // Catches a new link after the timeout joining the dead promise, or the old
  // answer writing after the new link's call.
  it('a new link after the timeout makes a new call and the old answer writes nothing', async () => {
    const first = deferred();
    const second = deferred();
    h.invokeSpy.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    void resolveLinkedCategory('1');
    await vi.advanceTimersByTimeAsync(6000);
    forgetFailedLinkedCategory('1');
    void resolveLinkedCategory('1');
    expect(h.invokeSpy).toHaveBeenCalledTimes(2);
    second.resolve(found('1', 'Fresh Name'));
    await settle();
    first.resolve(found('1', 'Stale Name'));
    await settle();
    expect(getLinkedCategory('1')).toMatchObject({ category: { name: 'Fresh Name' } });
  });

  // Catches a forget that issues a new token on a loading record: the first
  // call's late found would then be refused and the header would stay failed.
  it('a repeat link at 2 s leaves the resolve alone and the 8 s found still fills', async () => {
    const d = deferred();
    h.invokeSpy.mockReturnValue(d.promise);
    const late = vi.fn();
    onLateLinkedCategory(late);
    void resolveLinkedCategory('1');
    await vi.advanceTimersByTimeAsync(2000);
    forgetFailedLinkedCategory('1');
    void resolveLinkedCategory('1');
    expect(h.invokeSpy).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(4000);
    expect(getLinkedCategory('1')).toEqual({ state: 'failed' });
    await vi.advanceTimersByTimeAsync(2000);
    d.resolve(found('1', 'Late Name'));
    await settle();
    expect(getLinkedCategory('1')).toMatchObject({ state: 'found', category: { name: 'Late Name' } });
    expect(late).toHaveBeenCalledTimes(1);
  });

  // Catches a timer that stays armed after the answer: it would store failed
  // over a found record.
  it('an answer at 5 s is stored and the timer later changes nothing', async () => {
    const d = deferred();
    h.invokeSpy.mockReturnValue(d.promise);
    void resolveLinkedCategory('1');
    await vi.advanceTimersByTimeAsync(5000);
    d.resolve(found('1', 'Rust'));
    await settle();
    await vi.advanceTimersByTimeAsync(5000);
    expect(getLinkedCategory('1')?.state).toBe('found');
    expect(linkedCategorySizesForTest().inFlight).toBe(0);
  });

  // Catches an in-flight entry that survives a timeout: the next resolve would
  // join the dead promise and make no call.
  it('a hung call fails at 6 s and a retry then makes a new call that stores found', async () => {
    const first = deferred();
    h.invokeSpy.mockReturnValueOnce(first.promise).mockResolvedValueOnce(found('1', 'Fresh Name'));
    void resolveLinkedCategory('1');
    await vi.advanceTimersByTimeAsync(6000);
    expect(linkedCategorySizesForTest().inFlight).toBe(0);
    await resolveLinkedCategory('1', { retry: true });
    expect(h.invokeSpy).toHaveBeenCalledTimes(2);
    first.resolve(found('1', 'Stale Name'));
    await settle();
    expect(getLinkedCategory('1')).toMatchObject({ category: { name: 'Fresh Name' } });
  });

  // Catches a reset that drops the store's late listener, or an answer from a
  // call made before the reset writing after it.
  it('a reset keeps the late listeners and ignores answers from before it', async () => {
    const before = deferred();
    const after = deferred();
    h.invokeSpy.mockReturnValueOnce(before.promise).mockReturnValueOnce(after.promise);
    const late = vi.fn();
    onLateLinkedCategory(late);
    void resolveLinkedCategory('1');
    resetLinkedCategories();
    before.resolve(found('1', 'Before Reset'));
    await settle();
    expect(getLinkedCategory('1')).toBeUndefined();
    void resolveLinkedCategory('2');
    await vi.advanceTimersByTimeAsync(6000);
    after.resolve(found('2', 'After Reset'));
    await settle();
    expect(late).toHaveBeenCalledWith('2', expect.objectContaining({ name: 'After Reset' }));
  });

  // Catches a listener that can be called after it unsubscribed, or one that
  // breaks the rest when it throws.
  it('late listeners can be removed and one that throws does not stop the others', async () => {
    const d = deferred();
    h.invokeSpy.mockReturnValue(d.promise);
    const removed = vi.fn();
    const throwing = vi.fn(() => {
      throw new Error('listener');
    });
    const kept = vi.fn();
    onLateLinkedCategory(removed)();
    onLateLinkedCategory(throwing);
    onLateLinkedCategory(kept);
    void resolveLinkedCategory('1');
    await vi.advanceTimersByTimeAsync(6000);
    d.resolve(found('1', 'Late'));
    await settle();
    expect(removed).not.toHaveBeenCalled();
    expect(kept).toHaveBeenCalledTimes(1);
  });
});

describe('the background re-ask', () => {
  async function foundAt(id: string, name: string) {
    h.invokeSpy.mockResolvedValueOnce(found(id, name));
    await resolveLinkedCategory(id);
  }

  // Catches a stale found that blocks the screen with a loading state, or one
  // that does not ask again at all.
  it('a found record older than 1 hour answers at once, asks once and stays found', async () => {
    await foundAt('1', 'Old Name');
    const old = getLinkedCategory('1');
    vi.setSystemTime(1_000_000 + ONE_HOUR + 1);
    const d = deferred();
    h.invokeSpy.mockReturnValueOnce(d.promise);
    await expect(resolveLinkedCategory('1')).resolves.toBe(old);
    expect(h.invokeSpy).toHaveBeenCalledTimes(2);
    expect(getLinkedCategory('1')?.state).toBe('found');
    // Deduped like any resolve: a second look makes no third call.
    await resolveLinkedCategory('1');
    expect(h.invokeSpy).toHaveBeenCalledTimes(2);
    d.resolve(found('1', 'New Name'));
    await settle();
    expect(getLinkedCategory('1')).toMatchObject({
      state: 'found',
      category: { name: 'New Name' },
      at: 1_000_000 + ONE_HOUR + 1,
    });
  });

  // Catches an age check that is off by one in either direction.
  it('a record exactly 1 hour old or younger makes no call', async () => {
    await foundAt('1', 'Name');
    vi.setSystemTime(1_000_000 + ONE_HOUR);
    await resolveLinkedCategory('1');
    expect(h.invokeSpy).toHaveBeenCalledTimes(1);
  });

  // Catches a re-ask that replaces a good name with a failure or a miss.
  it('a re-ask that fails, is missing or is unreachable leaves the old found', async () => {
    for (const reply of [
      () => Promise.reject(new Error('x')),
      () => Promise.resolve({ state: 'missing' }),
      () => Promise.resolve({ state: 'unreachable' }),
      () => Promise.resolve({ nonsense: true }),
    ]) {
      resetLinkedCategories();
      h.invokeSpy.mockReset();
      vi.setSystemTime(1_000_000);
      await foundAt('1', 'Old Name');
      const old = getLinkedCategory('1');
      vi.setSystemTime(1_000_000 + ONE_HOUR + 5);
      h.invokeSpy.mockImplementationOnce(reply);
      void resolveLinkedCategory('1');
      await settle();
      expect(getLinkedCategory('1')).toBe(old);
      expect(linkedCategorySizesForTest().inFlight).toBe(0);
    }
  });

  // Catches a re-ask timer that stores failed over a good record, or leaves its
  // in-flight entry behind.
  it('a re-ask that hangs past 6 s writes no record and leaves no entry', async () => {
    await foundAt('1', 'Old Name');
    const old = getLinkedCategory('1');
    vi.setSystemTime(1_000_000 + ONE_HOUR + 5);
    h.invokeSpy.mockReturnValueOnce(new Promise(() => {}));
    void resolveLinkedCategory('1');
    const listener = vi.fn();
    subscribeLinkedCategories(listener);
    await vi.advanceTimersByTimeAsync(6000);
    expect(getLinkedCategory('1')).toBe(old);
    expect(linkedCategorySizesForTest().inFlight).toBe(0);
    expect(listener).not.toHaveBeenCalled();
  });

  // Catches a later resolve that joins a hung re-ask's dead promise: after its
  // 6 s the old record is stale again and asks once more.
  it('after a hung re-ask expires, a later resolve asks again', async () => {
    await foundAt('1', 'Old Name');
    vi.setSystemTime(1_000_000 + ONE_HOUR + 5);
    h.invokeSpy.mockReturnValueOnce(new Promise(() => {}));
    void resolveLinkedCategory('1');
    await vi.advanceTimersByTimeAsync(6000);
    h.invokeSpy.mockResolvedValueOnce(found('1', 'Newest'));
    void resolveLinkedCategory('1');
    await settle();
    expect(h.invokeSpy).toHaveBeenCalledTimes(3);
    expect(getLinkedCategory('1')).toMatchObject({ category: { name: 'Newest' } });
  });
});

describe('the record cap', () => {
  function answerEvery(hung: ReadonlySet<string> = new Set()) {
    h.invokeSpy.mockImplementation((_cmd: string, args: { id: string }) =>
      hung.has(args.id) ? new Promise(() => {}) : Promise.resolve(found(args.id, `Name ${args.id}`)),
    );
  }

  // Catches a cap that is off by one or that evicts the newest record.
  it('holds 64 ids and the 65th evicts the oldest', async () => {
    answerEvery();
    for (let i = 1; i <= 64; i++) await resolveLinkedCategory(String(i));
    expect(linkedCategorySizesForTest().records).toBe(64);
    expect(getLinkedCategory('1')).toBeDefined();
    await resolveLinkedCategory('65');
    expect(linkedCategorySizesForTest().records).toBe(64);
    expect(getLinkedCategory('1')).toBeUndefined();
    expect(getLinkedCategory('65')).toBeDefined();
  });

  // Catches a read that reorders the LRU: a render must not change the store it
  // reads. A resolve call, by contrast, is a use and moves its id to the end.
  it('reads leave the order alone and a resolve call moves the id to the end', async () => {
    answerEvery();
    for (let i = 1; i <= 64; i++) await resolveLinkedCategory(String(i));
    getLinkedCategory('1');
    renderToStaticMarkup(createElement(Probe, { id: '1' }));
    await resolveLinkedCategory('65');
    expect(getLinkedCategory('1')).toBeUndefined();
    expect(getLinkedCategory('2')).toBeDefined();

    resetLinkedCategories();
    for (let i = 1; i <= 64; i++) await resolveLinkedCategory(String(i));
    await resolveLinkedCategory('1');
    await resolveLinkedCategory('65');
    expect(getLinkedCategory('1')).toBeDefined();
    expect(getLinkedCategory('2')).toBeUndefined();
  });

  // Catches an eviction that runs once: with every candidate in flight at the
  // 65th write the map stays over the cap for good unless a later settle trims.
  it('64 hung calls and a 65th hold 65 until the oldest hung call settles, then 64', async () => {
    const hung = new Set(Array.from({ length: 64 }, (_, i) => String(i + 1)));
    answerEvery(hung);
    void resolveLinkedCategory('1');
    await vi.advanceTimersByTimeAsync(100);
    for (let i = 2; i <= 64; i++) void resolveLinkedCategory(String(i));
    await resolveLinkedCategory('65');
    expect(linkedCategorySizesForTest().records).toBe(65);
    // Only the oldest call's 6 s timer has fired at 6000; the rest fire at 6100.
    await vi.advanceTimersByTimeAsync(5900);
    expect(getLinkedCategory('1')).toEqual({ state: 'failed' });
    expect(getLinkedCategory('2')).toEqual({ state: 'loading' });
    expect(linkedCategorySizesForTest().records).toBe(64);
    expect(getLinkedCategory('65')).toBeUndefined();
  });

  // Catches an eviction of an id whose re-ask is in flight, or a settle that
  // deletes another try's entry: the record would end loading or be lost.
  it('an id with a re-ask in flight is not evicted, and its answer ends found', async () => {
    answerEvery();
    await resolveLinkedCategory('A');
    vi.setSystemTime(1_000_000 + ONE_HOUR + 1);
    const reask = deferred();
    h.invokeSpy.mockReturnValueOnce(reask.promise);
    void resolveLinkedCategory('A');
    expect(h.invokeSpy).toHaveBeenCalledTimes(2);
    answerEvery();
    for (let i = 1; i <= 64; i++) await resolveLinkedCategory(`B${i}`);
    expect(linkedCategorySizesForTest().records).toBe(64);
    expect(getLinkedCategory('A')?.state).toBe('found');
    expect(getLinkedCategory('B1')).toBeUndefined();
    expect(getLinkedCategory('B2')).toBeDefined();
    // A new link for the id while its re-ask is still running.
    forgetFailedLinkedCategory('A');
    await resolveLinkedCategory('A');
    expect(h.invokeSpy).toHaveBeenCalledTimes(2 + 64);
    reask.resolve(found('A', 'Fresh Name'));
    await settle();
    expect(getLinkedCategory('A')).toMatchObject({ state: 'found', category: { name: 'Fresh Name' } });
    expect(linkedCategorySizesForTest().inFlight).toBe(0);
  });

  // Catches a token sweep that runs only while the map is over its cap: each
  // forgotten id would leave a token behind for good.
  it('a forgotten record leaves no token behind after the next write', async () => {
    h.invokeSpy.mockRejectedValue(new Error('ipc'));
    for (let i = 1; i <= 10; i++) {
      await resolveLinkedCategory(`f${i}`);
      forgetFailedLinkedCategory(`f${i}`);
    }
    expect(linkedCategorySizesForTest().records).toBe(0);
    h.invokeSpy.mockReset();
    h.invokeSpy.mockResolvedValue(found('x', 'X'));
    await resolveLinkedCategory('x');
    const sizes = linkedCategorySizesForTest();
    expect(sizes).toEqual({ records: 1, tokens: 1, inFlight: 0 });
    expect(sizes.tokens).toBe(sizes.records + sizes.inFlight);
  });

  // Catches an eviction that keeps the evicted id's token.
  it('an evicted id takes its token with it', async () => {
    answerEvery();
    for (let i = 1; i <= 65; i++) await resolveLinkedCategory(String(i));
    const sizes = linkedCategorySizesForTest();
    expect(sizes.records).toBe(64);
    expect(sizes.tokens).toBe(64);
  });
});

function Probe({ id }: { id: string | null }) {
  const record = useLinkedCategory(id);
  return createElement('span', null, record?.state ?? 'none');
}

describe('useLinkedCategory and subscribers', () => {
  // Catches a hook with no server snapshot: React throws when it renders on the
  // server, and a screen reading the record needs one stable value either way.
  it('renders the record through a server snapshot', async () => {
    expect(renderToStaticMarkup(createElement(Probe, { id: '1' }))).toBe('<span>none</span>');
    expect(renderToStaticMarkup(createElement(Probe, { id: null }))).toBe('<span>none</span>');
    h.invokeSpy.mockResolvedValue(found('1', 'Rust'));
    await resolveLinkedCategory('1');
    expect(renderToStaticMarkup(createElement(Probe, { id: '1' }))).toBe('<span>found</span>');
  });

  // Catches a snapshot that changes identity without a write, which would loop a render.
  it('returns the same record object until it is written again', async () => {
    h.invokeSpy.mockResolvedValue(found('1', 'Rust'));
    await resolveLinkedCategory('1');
    expect(getLinkedCategory('1')).toBe(getLinkedCategory('1'));
  });

  // Catches a store that writes without telling the screens that read it.
  it('notifies on every write and keeps subscribers across a reset', async () => {
    const listener = vi.fn();
    const unsubscribe = subscribeLinkedCategories(listener);
    h.invokeSpy.mockResolvedValue(found('1', 'Rust'));
    await resolveLinkedCategory('1');
    // The loading write and the found write.
    expect(listener).toHaveBeenCalledTimes(2);
    resetLinkedCategories();
    expect(listener).toHaveBeenCalledTimes(3);
    unsubscribe();
    resetLinkedCategories();
    expect(listener).toHaveBeenCalledTimes(3);
  });
});

describe('categoryAnswerOutcome', () => {
  const record: LinkedCategory = {
    state: 'found',
    category: { id: '1', name: 'Rust', box_art_url: 'art' },
    at: 5,
  };

  // Catches a fill that loses the copy's own fields or skips the art.
  it('fills a nameless copy of the same id with the name and the art', () => {
    expect(categoryAnswerOutcome({ id: '1', name: '' }, '1', record)).toEqual({
      id: '1',
      name: 'Rust',
      box_art_url: 'art',
    });
  });

  // Catches a fill that overwrites a named copy, fills another id's copy or
  // reads a record that is not a found one.
  it('gives null for every other case', () => {
    expect(categoryAnswerOutcome({ id: '2', name: '' }, '1', record)).toBeNull();
    expect(categoryAnswerOutcome(null, '1', record)).toBeNull();
    expect(categoryAnswerOutcome({ id: '1', name: 'Already' }, '1', record)).toBeNull();
    expect(categoryAnswerOutcome({ id: '1', name: '' }, '1', { state: 'missing' })).toBeNull();
    expect(categoryAnswerOutcome({ id: '1', name: '' }, '1', { state: 'failed' })).toBeNull();
    expect(categoryAnswerOutcome({ id: '1', name: '' }, '1', { state: 'loading' })).toBeNull();
    expect(categoryAnswerOutcome({ id: '1', name: '' }, '1', undefined)).toBeNull();
    expect(categoryAnswerOutcome({ id: '1', name: '' }, '1', { state: 'found' })).toBeNull();
  });
});

describe('where the lookup is called from', () => {
  const srcRoot = resolve(__dirname, '..');
  const files: string[] = [];
  const walkDir = (dir: string) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) {
        if (entry.name !== 'node_modules') walkDir(path);
      } else if (/\.tsx?$/.test(entry.name)) {
        files.push(path);
      }
    }
  };
  walkDir(srcRoot);
  const posix = (path: string) => relative(srcRoot, path).split('\\').join('/');
  const name = 'resolve_' + 'category_link';
  // A call site: the word invoke, an optional type argument, then the quoted name.
  const callSite = new RegExp("(?<![.\\w])invoke(?:<[^()]*>)?\\(\\s*['\"`]" + name);

  // Catches a second place that calls the lookup: the drain, the stores and
  // the shells must read answers from this module, never ask Rust themselves.
  it('only linkedCategory.ts calls it, and no other source file spells it', () => {
    expect(files.length).toBeGreaterThan(0);
    const calls = files.filter((f) => callSite.test(readFileSync(f, 'utf8'))).map(posix);
    expect(calls).toEqual(['utils/linkedCategory.ts']);
    const spelled = files
      .filter((f) => !/\.test\.tsx?$/.test(f))
      .filter((f) => readFileSync(f, 'utf8').includes(name))
      .map(posix);
    expect(spelled).toEqual(['utils/linkedCategory.ts']);
  });

  // Catches a hook that loses its server snapshot in a refactor.
  it('useLinkedCategory passes a getServerSnapshot', () => {
    const source = readFileSync(resolve(__dirname, 'linkedCategory.ts'), 'utf8');
    expect(source).toContain('useSyncExternalStore(subscribeLinkedCategories, readRecord, readRecord)');
  });
});
