import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createStore } from 'zustand/vanilla';

const h = vi.hoisted(() => ({ main: true }));

vi.mock('./platform', () => ({
  get IS_MAIN_WINDOW() {
    return h.main;
  },
  isMainWindow: () => h.main,
}));

import {
  isAutomatic,
  lastSeenLinkStamp,
  noteBootAction,
  noteLinkStamp,
  noticeWorkSettled,
  opensPageOverPlayer,
  reportActivated,
  reportTaken,
  resetNoticeReportForTest,
  runAsAutomatic,
  trackLinkPeek,
  trackNoticeWork,
  watchBootActions,
  type InvokeFn,
  type PageWatchState,
} from './noticeReport';
import { startLinkDrain, type InboundLinkTarget } from './inboundLinkOpen';
import type { NoticeAction } from './notificationMirror';

const source = readFileSync(resolve(__dirname, 'noticeReport.ts'), 'utf8');

const flush = () => new Promise<void>((done) => setTimeout(done, 0));

function deferred<T = void>() {
  let resolveIt: (value: T) => void = () => {};
  let rejectIt: (error: unknown) => void = () => {};
  const promise = new Promise<T>((res, rej) => {
    resolveIt = res;
    rejectIt = rej;
  });
  return { promise, resolve: resolveIt, reject: rejectIt };
}

const watchLink = (key: string): InboundLinkTarget => ({ kind: 'watch', key });
const dropsAction: NoticeAction = { type: 'drops' };

// A stand-in for Rust's PendingLink: one slot, one counter shared by link
// stores and notice clicks, and a drop that only clears an older link.
function makeSlot(startAfter = 0) {
  let counter = startAfter;
  let held: { link: InboundLinkTarget; at: number; stamp: number } | null = null;
  return {
    click: () => ++counter,
    store(link: InboundLinkTarget, at = 1) {
      const stamp = ++counter;
      held = { link, at, stamp };
      return stamp;
    },
    take: async (): Promise<unknown> => {
      const value = held;
      held = null;
      return value ? { link: value.link, at: value.at } : null;
    },
    dropIfOlder(seq: number) {
      if (held && held.stamp < seq) held = null;
    },
    get held() {
      return held;
    },
  };
}

// A fake `invoke` that applies Rust's rules to the slot, and records calls.
function rustInvoke(slot: ReturnType<typeof makeSlot>) {
  const calls: Array<{ cmd: string; args?: Record<string, unknown> }> = [];
  const fn: InvokeFn = async (cmd, args) => {
    calls.push({ cmd, args });
    if (cmd === 'link_superseded') slot.dropIfOlder(Number(args?.seen) + 1);
    if (cmd === 'notice_opened') slot.dropIfOlder(Number(args?.seq));
    return null;
  };
  return { fn, calls, supersededCalls: () => calls.filter((c) => c.cmd === 'link_superseded') };
}

beforeEach(() => {
  resetNoticeReportForTest();
  h.main = true;
});

afterEach(() => {
  vi.useRealTimers();
  resetNoticeReportForTest();
});

describe('reportActivated', () => {
  // Catches a report with no closure behind it: a dead click opens nothing and
  // must not clear a good link.
  it('runs a stored closure once and reports once, in that order', () => {
    const order: string[] = [];
    const actions = new Map<string, () => void>([['n1', () => order.push('run')]]);
    reportActivated({ id: 'n1', seq: 9 }, actions, (seq) => order.push(`report:${seq}`));
    expect(order).toEqual(['run', 'report:9']);
    expect(actions.has('n1')).toBe(false);
  });

  // Catches a click for an id the page never held, or one already consumed,
  // reporting anyway.
  it('reports nothing without a closure, and nothing the second time', () => {
    const report = vi.fn();
    const run = vi.fn();
    const actions = new Map<string, () => void>([['n1', run]]);
    reportActivated({ id: 'missing', seq: 1 }, actions, report);
    reportActivated({ id: 'n1', seq: 2 }, actions, report);
    reportActivated({ id: 'n1', seq: 3 }, actions, report);
    expect(run).toHaveBeenCalledTimes(1);
    expect(report).toHaveBeenCalledTimes(1);
    expect(report).toHaveBeenCalledWith(2);
  });

  // Catches the stamp order in Rust being ignored: a link stored before the
  // click is dropped, a link stored after it stays.
  it('drops a link stored before the click and keeps one stored after it', () => {
    const before = makeSlot(4);
    before.store(watchLink('older'));
    const click = before.click();
    const rust = rustInvoke(before);
    reportActivated(
      { id: 'n', seq: click },
      new Map([['n', () => {}]]),
      (seq) => void rust.fn('notice_opened', { seq }),
    );
    expect(before.held).toBeNull();

    const after = makeSlot(4);
    const seq = after.click();
    after.store(watchLink('newer'));
    const rust2 = rustInvoke(after);
    reportActivated({ id: 'n', seq }, new Map([['n', () => {}]]), (s) => void rust2.fn('notice_opened', { seq: s }));
    expect(after.held?.link).toEqual(watchLink('newer'));
  });

  // Catches a dead click that reports: the older link must survive it.
  it('a click whose closure is missing never reports and the link survives', () => {
    const slot = makeSlot(4);
    slot.store(watchLink('keep'));
    const seq = slot.click();
    const rust = rustInvoke(slot);
    reportActivated({ id: 'gone', seq }, new Map(), (s) => void rust.fn('notice_opened', { seq: s }));
    expect(rust.calls).toHaveLength(0);
    expect(slot.held?.link).toEqual(watchLink('keep'));
  });
});

describe('reportTaken', () => {
  // Catches a take of nothing reporting, or running anything.
  it('calls nothing for null', () => {
    const run = vi.fn();
    const report = vi.fn();
    expect(reportTaken(null, run, report)).toBeUndefined();
    expect(run).not.toHaveBeenCalled();
    expect(report).not.toHaveBeenCalled();
  });

  // Catches a report tied to the take instead of to the open: a settings tab
  // that is not allowed, or a deferred watch, must report only when it opens.
  it('reports exactly when run calls opened', () => {
    const report = vi.fn();
    reportTaken({ action: dropsAction, seq: 7 }, (_action, opened) => opened(), report);
    expect(report).toHaveBeenCalledTimes(1);
    expect(report).toHaveBeenCalledWith(7);

    const none = vi.fn();
    reportTaken({ action: { type: 'settings', tab: 'Nope' }, seq: 8 }, () => {}, none);
    expect(none).not.toHaveBeenCalled();

    const later = vi.fn();
    let go: () => void = () => {};
    reportTaken({ action: dropsAction, seq: 9 }, (_action, opened) => void (go = opened), later);
    expect(later).not.toHaveBeenCalled();
    go();
    expect(later).toHaveBeenCalledWith(9);
  });

  // Catches a take that swallows what run returns: a deferred open would then
  // fall out of the tracked work and the post-boot take would not wait for it.
  it('returns exactly what run returns', async () => {
    const d = deferred();
    const result = reportTaken({ action: dropsAction, seq: 1 }, () => d.promise, vi.fn());
    expect(result).toBe(d.promise);
    d.resolve();
    await result;
  });
});

describe('trackNoticeWork and noticeWorkSettled', () => {
  async function isDone(p: Promise<void>): Promise<boolean> {
    let done = false;
    void p.then(() => {
      done = true;
    });
    await flush();
    return done;
  }

  // Catches a tracker that wraps the promise: callers keep the original.
  it('returns the very promise it was given', () => {
    const p = Promise.resolve(1);
    expect(trackNoticeWork(p)).toBe(p);
  });

  // Catches a wait that returns before the tracked work settles, or one that
  // never leaves the set after it does.
  it('waits for tracked work and is done once it settles', async () => {
    expect(await isDone(noticeWorkSettled())).toBe(true);
    const d = deferred();
    trackNoticeWork(d.promise);
    const waiting = noticeWorkSettled();
    expect(await isDone(waiting)).toBe(false);
    d.resolve();
    expect(await isDone(waiting)).toBe(true);
    expect(await isDone(noticeWorkSettled())).toBe(true);
  });

  // Catches a single pass: a report made by a run that a tracked take started
  // is a new entry, and the take after boot must wait for it too.
  it('waits for an entry that a finished entry added', async () => {
    const take = deferred();
    const report = deferred();
    trackNoticeWork(
      take.promise.then(() => {
        trackNoticeWork(report.promise);
      }),
    );
    const waiting = noticeWorkSettled();
    take.resolve();
    expect(await isDone(waiting)).toBe(false);
    report.resolve();
    expect(await isDone(waiting)).toBe(true);
  });

  // Catches a rejected report blocking the take, and a rejection of the
  // tracker's own making (a bare then(del) would leave one unhandled).
  it('a rejected entry leaves the set with no unhandled rejection', async () => {
    const unhandled: unknown[] = [];
    const listener = (reason: unknown) => void unhandled.push(reason);
    process.on('unhandledRejection', listener);
    try {
      const p = Promise.reject(new Error('report failed'));
      p.catch(() => {});
      trackNoticeWork(p);
      expect(await isDone(noticeWorkSettled())).toBe(true);
      await flush();
      expect(unhandled).toEqual([]);
    } finally {
      process.off('unhandledRejection', listener);
    }
  });

  // Catches a hung entry holding every later link for good.
  it('work that never settles lets the wait through at 3 s', async () => {
    vi.useFakeTimers();
    trackNoticeWork(new Promise<void>(() => {}));
    let done = false;
    void noticeWorkSettled().then(() => {
      done = true;
    });
    await vi.advanceTimersByTimeAsync(2999);
    expect(done).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    expect(done).toBe(true);
  });

  // Catches a hung entry that stays in the set: the first wait takes at most
  // 3 s and a later one, 10 s on, waits for nothing.
  it('one hung entry then two waits 10 s apart: the second does not wait', async () => {
    vi.useFakeTimers();
    trackNoticeWork(new Promise<void>(() => {}));
    let first = false;
    void noticeWorkSettled().then(() => {
      first = true;
    });
    await vi.advanceTimersByTimeAsync(3000);
    expect(first).toBe(true);
    await vi.advanceTimersByTimeAsync(7000);
    let second = false;
    void noticeWorkSettled().then(() => {
      second = true;
    });
    await vi.advanceTimersByTimeAsync(0);
    expect(second).toBe(true);
  });

  // Catches a loop with no bound: work that keeps adding work must not hold a
  // take past the one 3 s limit.
  it('endless new work still lets the wait through at 3 s', async () => {
    vi.useFakeTimers();
    const again = (): Promise<void> =>
      trackNoticeWork(new Promise<void>((res) => setTimeout(res, 1000))).then(() => void again());
    void again();
    let done = false;
    void noticeWorkSettled().then(() => {
      done = true;
    });
    await vi.advanceTimersByTimeAsync(2999);
    expect(done).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    expect(done).toBe(true);
  });
});

describe('runAsAutomatic', () => {
  // Catches a flag that is never set, or one that returns the wrong value.
  it('returns the value and sets the flag only inside', () => {
    expect(isAutomatic()).toBe(false);
    const seen = runAsAutomatic(() => isAutomatic());
    expect(seen).toBe(true);
    expect(isAutomatic()).toBe(false);
    expect(runAsAutomatic(() => 42)).toBe(42);
  });

  // Catches a throwing fn leaving the flag set: every later person action would
  // be ignored for the rest of boot.
  it('a throwing fn clears the flag and a plain change then invokes again', () => {
    const slot = makeSlot();
    const rust = rustInvoke(slot);
    noteLinkStamp(3);
    expect(() =>
      runAsAutomatic(() => {
        throw new Error('boom');
      }),
    ).toThrow('boom');
    expect(isAutomatic()).toBe(false);
    noteBootAction(rust.fn);
    return flush().then(() => expect(rust.supersededCalls()).toHaveLength(1));
  });

  // Catches a nested call clearing the outer flag when the inner one returns.
  it('a nested call leaves the outer flag set', () => {
    runAsAutomatic(() => {
      runAsAutomatic(() => {});
      expect(isAutomatic()).toBe(true);
    });
    expect(isAutomatic()).toBe(false);
  });
});

describe('noteLinkStamp and lastSeenLinkStamp', () => {
  // Catches a stamp that goes backwards: an older event arriving late must not
  // lower what the page has seen.
  it('keeps the highest stamp and starts at 0', () => {
    expect(lastSeenLinkStamp()).toBe(0);
    noteLinkStamp(5);
    noteLinkStamp(3);
    noteLinkStamp(Number.NaN);
    expect(lastSeenLinkStamp()).toBe(5);
    noteLinkStamp(8);
    expect(lastSeenLinkStamp()).toBe(8);
  });
});

describe('noteBootAction', () => {
  // Catches an action that names no stamp or the wrong one, and a step that is
  // not tracked (the post-boot take would not wait for it).
  it('invokes link_superseded with the stamp seen, tracked', async () => {
    noteLinkStamp(5);
    const d = deferred();
    const invokeSpy = vi.fn(() => d.promise.then(() => null));
    noteBootAction(invokeSpy as unknown as InvokeFn);
    await flush();
    expect(invokeSpy).toHaveBeenCalledTimes(1);
    expect(invokeSpy).toHaveBeenCalledWith('link_superseded', { seen: 5 });
    let waited = false;
    void noticeWorkSettled().then(() => {
      waited = true;
    });
    await flush();
    expect(waited).toBe(false);
    d.resolve();
    await flush();
    expect(waited).toBe(true);
  });

  // Catches an automatic write counting as a person's action.
  it('does nothing while the automatic flag is set', async () => {
    noteLinkStamp(5);
    const invokeSpy = vi.fn(async () => null);
    runAsAutomatic(() => noteBootAction(invokeSpy));
    await flush();
    expect(invokeSpy).not.toHaveBeenCalled();
  });

  // Catches an invoke with seen 0, which would drop a link the page never heard of.
  it('invokes nothing while the stamp stays 0, with or without a peek', async () => {
    const invokeSpy = vi.fn(async () => null);
    noteBootAction(invokeSpy);
    await flush();
    trackLinkPeek(Promise.resolve(null));
    noteBootAction(invokeSpy);
    await flush();
    expect(invokeSpy).not.toHaveBeenCalled();
  });

  // Catches an action right after mount that reads 0 and gives up before the
  // mount peek has answered.
  it('with stamp 0 and a peek pending it waits for the peek, then drops the link it names', async () => {
    const slot = makeSlot();
    const rust = rustInvoke(slot);
    const peek = deferred<number>();
    const tracked = trackLinkPeek(peek.promise);
    void tracked.then(noteLinkStamp);
    slot.store(watchLink('before-listener'));
    noteBootAction(rust.fn);
    await flush();
    expect(rust.calls).toHaveLength(0);
    peek.resolve(1);
    await flush();
    expect(rust.supersededCalls()).toEqual([{ cmd: 'link_superseded', args: { seen: 1 } }]);
    expect(slot.held).toBeNull();
  });

  // Catches a rejecting peek that rejects the tracked step or skips the
  // re-read: an event stamp that arrived meanwhile must still be used.
  it('a rejecting peek raises nothing and the stamp is read again', async () => {
    const unhandled: unknown[] = [];
    const listener = (reason: unknown) => void unhandled.push(reason);
    process.on('unhandledRejection', listener);
    try {
      const invokeSpy = vi.fn(async () => null);
      const peek = deferred<number>();
      trackLinkPeek(peek.promise).catch(() => {});
      noteBootAction(invokeSpy);
      noteLinkStamp(7);
      peek.reject(new Error('no peek'));
      await flush();
      await flush();
      expect(invokeSpy).toHaveBeenCalledTimes(1);
      expect(invokeSpy).toHaveBeenCalledWith('link_superseded', { seen: 7 });
      expect(unhandled).toEqual([]);

      const quiet = vi.fn(async () => null);
      resetNoticeReportForTest();
      const peek2 = deferred<number>();
      trackLinkPeek(peek2.promise).catch(() => {});
      noteBootAction(quiet);
      peek2.reject(new Error('no peek'));
      await flush();
      await flush();
      expect(quiet).not.toHaveBeenCalled();
      expect(unhandled).toEqual([]);
    } finally {
      process.off('unhandledRejection', listener);
    }
  });

  // Catches a failing invoke that raises or blocks the take.
  it('an invoke that rejects raises nothing and the wait still ends', async () => {
    const unhandled: unknown[] = [];
    const listener = (reason: unknown) => void unhandled.push(reason);
    process.on('unhandledRejection', listener);
    try {
      noteLinkStamp(2);
      noteBootAction(async () => {
        throw new Error('older backend');
      });
      await noticeWorkSettled();
      await flush();
      expect(unhandled).toEqual([]);
    } finally {
      process.off('unhandledRejection', listener);
    }
  });

  // Catches an invoke that throws before it returns a promise.
  it('an invoke that throws raises nothing', async () => {
    noteLinkStamp(2);
    noteBootAction(() => {
      throw new Error('sync');
    });
    await noticeWorkSettled();
    await flush();
  });
});

describe('opensPageOverPlayer', () => {
  const flags = [
    'isSettingsOpen',
    'showBadgesOverlay',
    'showWhispersOverlay',
    'showEmoteSetsOverlay',
    'showDropsOverlay',
    'showLiveStreamsOverlay',
    'showMarketplaceOverlay',
    'showDashboardOverlay',
    'isCommandPaletteOpen',
  ] as const;
  const slots = [
    'profileViewerUserId',
    'songsPanel',
    'emoteSpotlight',
    'profileModalUser',
    'clipModal',
    'vodModal',
  ] as const;

  // Catches a layer missing from the watch: a Settings or Drops open during
  // boot would not supersede a waiting link.
  it.each(flags)('%s opening is a page, closing it is not', (flag) => {
    expect(opensPageOverPlayer({ [flag]: false }, { [flag]: true })).toBe(true);
    expect(opensPageOverPlayer({ [flag]: true }, { [flag]: false })).toBe(false);
    expect(opensPageOverPlayer({ [flag]: true }, { [flag]: true })).toBe(false);
  });

  it.each(slots)('%s going from empty to held is a page', (slot) => {
    expect(opensPageOverPlayer({ [slot]: null }, { [slot]: slot === 'profileViewerUserId' ? 'u1' : {} })).toBe(true);
    expect(opensPageOverPlayer({ [slot]: {} }, { [slot]: null })).toBe(false);
  });

  // Catches Home's page changes being missed, or a name fill counted as one.
  it("counts Home's tab, category, provider category and the platform scope", () => {
    expect(opensPageOverPlayer({ homeActiveTab: 'following' }, { homeActiveTab: 'browse' })).toBe(true);
    expect(
      opensPageOverPlayer({ homeSelectedCategory: { id: '1' } }, { homeSelectedCategory: { id: '2' } }),
    ).toBe(true);
    expect(opensPageOverPlayer({ homeSelectedCategory: null }, { homeSelectedCategory: { id: '2' } })).toBe(true);
    expect(
      opensPageOverPlayer({ homeProviderCategory: { id: '1' } }, { homeProviderCategory: { id: '2' } }),
    ).toBe(true);
    expect(opensPageOverPlayer({ activePlatform: 'all' }, { activePlatform: 'kick' })).toBe(true);
    // The same category filled with its name is the same page.
    expect(
      opensPageOverPlayer({ homeSelectedCategory: { id: '1' } }, { homeSelectedCategory: { id: '1' } }),
    ).toBe(false);
  });

  // Catches a field of another store creeping into this watch: a store watch
  // only sees the fields its own store declares.
  it('has no MultiNook term', () => {
    const other = { ['isMulti' + 'NookActive']: true } as PageWatchState;
    expect(opensPageOverPlayer({}, other)).toBe(false);
    expect(source).not.toContain('isMulti' + 'NookActive');
  });

  // Catches an unrelated change counting as a page.
  it('is false for anything else', () => {
    expect(opensPageOverPlayer({ isBooting: true }, { isBooting: false })).toBe(false);
    expect(opensPageOverPlayer({}, {})).toBe(false);
  });
});

describe('watchBootActions', () => {
  type TestState = PageWatchState & { isBooting: boolean; sidebarOpen: boolean; volume: number };
  const initial = (): TestState => ({
    isBooting: true,
    sidebarOpen: false,
    volume: 50,
    isSettingsOpen: false,
    showDropsOverlay: false,
    homeActiveTab: 'following',
    homeSelectedCategory: null,
    homeProviderCategory: null,
    activePlatform: 'all',
    clipModal: null,
  });

  // Catches a watch that never subscribes, or an invoke that names no stamp:
  // a Settings open during boot drops the link the page had heard of and
  // leaves Settings open, while a link stored after it still opens.
  it('a Settings open during boot drops the seen link; a link stored after it opens', async () => {
    const slot = makeSlot();
    const rust = rustInvoke(slot);
    const store = createStore<TestState>(initial);
    const stop = watchBootActions(store, rust.fn);
    const seen = slot.store(watchLink('older'));
    noteLinkStamp(seen);
    store.setState({ isSettingsOpen: true });
    await flush();
    expect(rust.supersededCalls()).toEqual([{ cmd: 'link_superseded', args: { seen } }]);
    expect(slot.held).toBeNull();
    expect(store.getState().isSettingsOpen).toBe(true);

    const after = slot.store(watchLink('newer'));
    expect(after).toBeGreaterThan(seen);
    store.setState({ isSettingsOpen: false });
    store.setState({ isSettingsOpen: true });
    await flush();
    // Still only the page's own stamp: the newer link was never heard of.
    expect(slot.held?.link).toEqual(watchLink('newer'));
    stop();
  });

  // Catches a link stored after the page heard of the old one being dropped by
  // the action (the stamp the page names is below it).
  it('names only the stamp the page has seen, so a later link survives', async () => {
    const slot = makeSlot();
    const rust = rustInvoke(slot);
    const store = createStore<TestState>(initial);
    watchBootActions(store, rust.fn);
    const seen = slot.store(watchLink('heard'));
    noteLinkStamp(seen);
    slot.store(watchLink('unheard'));
    store.setState({ showDropsOverlay: true });
    await flush();
    expect(slot.held?.link).toEqual(watchLink('unheard'));
  });

  // Catches a watch that treats a sidebar toggle or a volume change as a page.
  it('a sidebar toggle or a volume change drops nothing', async () => {
    const rust = rustInvoke(makeSlot());
    const store = createStore<TestState>(initial);
    watchBootActions(store, rust.fn);
    noteLinkStamp(3);
    store.setState({ sidebarOpen: true });
    store.setState({ volume: 10 });
    await flush();
    expect(rust.calls).toHaveLength(0);
  });

  // Catches a popout reporting: it has no part in the main window's boot.
  it('invokes nothing from a window that is not main', async () => {
    h.main = false;
    const rust = rustInvoke(makeSlot());
    const store = createStore<TestState>(initial);
    watchBootActions(store, rust.fn);
    noteLinkStamp(3);
    store.setState({ isSettingsOpen: true });
    await flush();
    expect(rust.calls).toHaveLength(0);
  });

  // Catches a watch that keeps running after boot, or after it was stopped.
  it('invokes nothing once boot is over or the watch is stopped', async () => {
    const rust = rustInvoke(makeSlot());
    const store = createStore<TestState>(initial);
    const stop = watchBootActions(store, rust.fn);
    noteLinkStamp(3);
    store.setState({ isBooting: false });
    store.setState({ isSettingsOpen: true });
    await flush();
    expect(rust.calls).toHaveLength(0);

    store.setState({ isBooting: true, isSettingsOpen: false });
    stop();
    store.setState({ isSettingsOpen: true });
    await flush();
    expect(rust.calls).toHaveLength(0);
  });

  // Catches each watched Home field being skipped.
  it('a platform change, a category change and a tab click each invoke once', async () => {
    const rust = rustInvoke(makeSlot());
    const store = createStore<TestState>(initial);
    watchBootActions(store, rust.fn);
    noteLinkStamp(4);
    store.setState({ activePlatform: 'kick' });
    store.setState({ homeProviderCategory: { id: 'k1' } });
    store.setState({ homeActiveTab: 'browse' });
    store.setState({ homeSelectedCategory: { id: '9' } });
    await flush();
    expect(rust.supersededCalls()).toHaveLength(4);
  });

  // Catches an automatic write (a restore, a default tab) counted as an action,
  // while a person's click on the same field still counts.
  it('a change inside runAsAutomatic invokes nothing; the same change made plainly invokes', async () => {
    const rust = rustInvoke(makeSlot());
    const store = createStore<TestState>(initial);
    watchBootActions(store, rust.fn);
    noteLinkStamp(4);
    runAsAutomatic(() => store.setState({ homeActiveTab: 'recommended' }));
    runAsAutomatic(() => store.setState({ activePlatform: 'twitch' }));
    await flush();
    expect(rust.calls).toHaveLength(0);
    store.setState({ homeActiveTab: 'following' });
    await flush();
    expect(rust.supersededCalls()).toHaveLength(1);
  });

  // Catches a start function's entry line (a plain noteBootAction call) being
  // ignored, and an automatic start being counted.
  it('a start called inside runAsAutomatic invokes nothing and one called plainly invokes once', async () => {
    const rust = rustInvoke(makeSlot());
    noteLinkStamp(4);
    runAsAutomatic(() => noteBootAction(rust.fn));
    await flush();
    expect(rust.calls).toHaveLength(0);
    noteBootAction(rust.fn);
    await flush();
    expect(rust.supersededCalls()).toHaveLength(1);
  });

  // Catches a link stored before the listener existed being forgotten: only the
  // peek knows its stamp, and an action right after it must drop that link.
  it('a link learned through the mount peek is dropped by a later action', async () => {
    const slot = makeSlot();
    const rust = rustInvoke(slot);
    const stamp = slot.store(watchLink('stored-before-listen'));
    const store = createStore<TestState>(initial);
    watchBootActions(store, rust.fn);
    const peek = trackLinkPeek(Promise.resolve(stamp));
    void peek.then(noteLinkStamp);
    await flush();
    store.setState({ isSettingsOpen: true });
    await flush();
    expect(rust.supersededCalls()).toEqual([{ cmd: 'link_superseded', args: { seen: stamp } }]);
    expect(slot.held).toBeNull();
  });
});

describe('notice work settles before the post-boot take', () => {
  // The drain's own listen: settled at once, nothing else needed.
  const listen = () => Promise.resolve(() => {});

  function startDrain(slot: ReturnType<typeof makeSlot>, ready: () => boolean) {
    const opened: InboundLinkTarget[] = [];
    const drain = startLinkDrain({
      listen,
      take: slot.take,
      open: (link) => void opened.push(link),
      ready,
      beforeTake: noticeWorkSettled,
    });
    return { drain, opened };
  }

  // Catches a take that does not wait for the report: the older link would be
  // opened before notice_opened cleared it.
  it('a late report clears the older link before the take', async () => {
    const slot = makeSlot(4);
    let ready = false;
    const { drain, opened } = startDrain(slot, () => ready);
    await flush();
    slot.store(watchLink('older'));
    const seq = slot.click();
    const answer = deferred();
    const report = (n: number) => {
      void trackNoticeWork(answer.promise.then(() => slot.dropIfOlder(n)));
    };
    reportActivated({ id: 'n', seq }, new Map([['n', () => {}]]), report);
    ready = true;
    drain.drain();
    await flush();
    expect(opened).toEqual([]);
    expect(slot.held).not.toBeNull();
    answer.resolve();
    await flush();
    expect(opened).toEqual([]);
    expect(slot.held).toBeNull();
  });

  // Catches a click newer than the link being treated as older: the link
  // stored after the click must still open.
  it('a link stored after the click still opens', async () => {
    const slot = makeSlot(4);
    let ready = false;
    const { drain, opened } = startDrain(slot, () => ready);
    await flush();
    const seq = slot.click();
    slot.store(watchLink('newer'));
    reportActivated({ id: 'n', seq }, new Map([['n', () => {}]]), (n) => slot.dropIfOlder(n));
    ready = true;
    drain.drain();
    await flush();
    expect(opened).toEqual([watchLink('newer')]);
  });

  // Catches a take_notice_action that lands after boot being left out of the
  // wait: its action opens something, so the older link must go first.
  it('a notice take that resolves after boot end still drops the older link first', async () => {
    const slot = makeSlot(4);
    let ready = false;
    const { drain, opened } = startDrain(slot, () => ready);
    await flush();
    slot.store(watchLink('older'));
    const seq = slot.click();
    const takeNotice = deferred<{ action: NoticeAction; seq: number } | null>();
    trackNoticeWork(
      takeNotice.promise.then((taken) =>
        reportTaken(
          taken,
          (_action, opened) => opened(),
          (n) => void trackNoticeWork(Promise.resolve().then(() => slot.dropIfOlder(n))),
        ),
      ),
    );
    ready = true;
    drain.drain();
    await flush();
    expect(opened).toEqual([]);
    takeNotice.resolve({ action: dropsAction, seq });
    await flush();
    expect(opened).toEqual([]);
    expect(slot.held).toBeNull();
  });

  // Catches a failed report holding the take for good.
  it('a rejected report does not block the take', async () => {
    const slot = makeSlot(4);
    let ready = false;
    const { drain, opened } = startDrain(slot, () => ready);
    await flush();
    slot.store(watchLink('stays'));
    const failing = Promise.reject(new Error('notice_opened failed'));
    trackNoticeWork(failing.catch(() => {}));
    ready = true;
    drain.drain();
    await flush();
    expect(opened).toEqual([watchLink('stays')]);
  });
});
