import { describe, expect, it, vi } from 'vitest';

const logs = vi.hoisted(() => ({ info: vi.fn() }));
vi.mock('./logger', () => ({ Logger: { debug: vi.fn(), info: logs.info, warn: vi.fn(), error: vi.fn() } }));
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import {
  INBOUND_LINK_EVENT,
  clearHeldLink,
  createPhoneLinkDoors,
  heldAt,
  holdLink,
  isOlder,
  openInboundLink,
  readCategoryAnswer,
  readInboundLink,
  readTakenLink,
  startLinkDrain,
  startPhoneLinkBridge,
  takeHeld,
  type InboundLinkDeps,
  type InboundLinkTarget,
} from './inboundLinkOpen';

const source = readFileSync(resolve(__dirname, 'inboundLinkOpen.ts'), 'utf8');

const flush = () => new Promise<void>((done) => setTimeout(done, 0));

const watch = (key: string): InboundLinkTarget => ({ kind: 'watch', key });
const badge = (setId: string, version = '1'): InboundLinkTarget => ({ kind: 'badge', setId, version });
const category = (id: string): InboundLinkTarget => ({ kind: 'category', provider: 'twitch', id });
const taken = (link: InboundLinkTarget, at = 1) => ({ link, at });

// The shared wire fixture Rust pins: every link, answer and `{ link, at }` object
// in it, wherever it sits, must read back unchanged.
function walk(node: unknown, visit: (object: Record<string, unknown>) => void): void {
  if (Array.isArray(node)) {
    node.forEach((item) => walk(item, visit));
  } else if (node && typeof node === 'object') {
    visit(node as Record<string, unknown>);
    Object.values(node).forEach((item) => walk(item, visit));
  }
}

describe('the shared link fixture', () => {
  const fixture: unknown = JSON.parse(
    readFileSync(resolve(__dirname, '../../src-tauri/tests/fixtures/inbound_link.json'), 'utf8'),
  );
  const links: Record<string, unknown>[] = [];
  const answers: Record<string, unknown>[] = [];
  const takens: Record<string, unknown>[] = [];
  walk(fixture, (object) => {
    if (typeof object.kind === 'string') links.push(object);
    if (typeof object.state === 'string') answers.push(object);
    if ('link' in object && typeof object.at === 'number') takens.push(object);
  });

  // Catches a reader that drifts from the JSON Rust writes: an empty scan or a
  // missing kind in the fixture fails here instead of passing for nothing.
  it('holds every kind of link, every answer and a taken link', () => {
    for (const kind of ['watch', 'badge', 'category']) {
      expect(links.some((l) => l.kind === kind)).toBe(true);
    }
    for (const state of ['found', 'missing', 'unreachable']) {
      expect(answers.some((a) => a.state === state)).toBe(true);
    }
    expect(takens.length).toBeGreaterThan(0);
  });

  // Catches a reader that renames or drops a field Rust sends.
  it('reads every link', () => {
    for (const link of links) {
      if (link.kind === 'category' && link.provider !== 'twitch') {
        expect(readInboundLink(link)).toBeNull();
      } else {
        expect(readInboundLink(link)).toEqual(link);
      }
    }
  });

  // Catches a category answer reader that renames a snake_case field.
  it('reads every category answer', () => {
    for (const answer of answers) {
      const read = readCategoryAnswer(answer);
      expect(read).not.toBeNull();
      expect(read?.state).toBe(answer.state);
      if (answer.state === 'found') {
        expect(read).toEqual({
          state: 'found',
          id: answer.id,
          name: answer.name,
          box_art_url: answer.box_art_url,
          viewer_count: answer.viewer_count ?? null,
        });
      }
    }
  });

  // Catches a taken-link reader that loses the boot-clock time.
  it('reads the taken link with its time', () => {
    for (const object of takens) {
      expect(readTakenLink(object)).toEqual({ link: readInboundLink(object.link), at: object.at });
    }
  });
});

describe('readInboundLink', () => {
  // Catches a badge link whose missing version stays undefined.
  it('defaults a badge link version to 1', () => {
    expect(readInboundLink({ kind: 'badge', setId: 'clover' })).toEqual(badge('clover', '1'));
    expect(readInboundLink({ kind: 'badge', setId: 'clover', version: '' })).toEqual(badge('clover', '1'));
    expect(readInboundLink({ kind: 'badge', setId: 'clover', version: 3 })).toEqual(badge('clover', '1'));
    expect(readInboundLink({ kind: 'badge', setId: 'clover', version: '3' })).toEqual(badge('clover', '3'));
  });

  // Catches a reader that lets a malformed link through to an opener.
  it('rejects anything that is not a link', () => {
    for (const bad of [
      null,
      undefined,
      'xqc',
      7,
      {},
      { kind: 'watch' },
      { kind: 'watch', key: '' },
      { kind: 'watch', key: 5 },
      { kind: 'badge', setId: '' },
      { kind: 'badge' },
      { kind: 'category', provider: 'kick', id: '15' },
      { kind: 'category', provider: 'twitch', id: '' },
      { kind: 'category', provider: 'twitch', id: 'abc' },
      { kind: 'category', provider: 'twitch', id: '1234567890123456' },
      { kind: 'category', provider: 'twitch', id: 509658 },
      { kind: 'category', id: '509658' },
      { kind: 'other', key: 'x' },
    ]) {
      expect(readInboundLink(bad)).toBeNull();
    }
  });

  // Catches an id check that is looser or stricter than 1 to 15 digits.
  it('accepts a category id of 1 to 15 digits', () => {
    expect(readInboundLink({ kind: 'category', provider: 'twitch', id: '1' })).toEqual(category('1'));
    expect(readInboundLink({ kind: 'category', provider: 'twitch', id: '123456789012345' })).toEqual(
      category('123456789012345'),
    );
  });

  // Catches a reader that passes unknown fields along to the opener.
  it('returns only the fields it knows', () => {
    expect(readInboundLink({ kind: 'watch', key: 'xqc', extra: 1 })).toEqual(watch('xqc'));
  });
});

describe('readTakenLink', () => {
  // Catches a take whose time is not a finite number being treated as a link.
  it('needs a finite at and a readable link', () => {
    expect(readTakenLink({ link: watch('xqc'), at: 86400000 })).toEqual(taken(watch('xqc'), 86400000));
    expect(readTakenLink({ link: watch('xqc'), at: 0 })).toEqual(taken(watch('xqc'), 0));
    expect(readTakenLink(watch('xqc'))).toBeNull();
    expect(readTakenLink({ link: watch('xqc') })).toBeNull();
    expect(readTakenLink({ link: watch('xqc'), at: '5' })).toBeNull();
    expect(readTakenLink({ link: watch('xqc'), at: Number.NaN })).toBeNull();
    expect(readTakenLink({ link: watch('xqc'), at: Number.POSITIVE_INFINITY })).toBeNull();
    expect(readTakenLink({ link: { kind: 'watch' }, at: 1 })).toBeNull();
    expect(readTakenLink(null)).toBeNull();
    expect(readTakenLink('x')).toBeNull();
  });
});

describe('readCategoryAnswer', () => {
  // Catches an answer reader that drops the viewer count or invents one.
  it('reads the three states', () => {
    expect(
      readCategoryAnswer({ state: 'found', id: '509658', name: 'Just Chatting', box_art_url: 'u', viewer_count: 12 }),
    ).toEqual({ state: 'found', id: '509658', name: 'Just Chatting', box_art_url: 'u', viewer_count: 12 });
    expect(readCategoryAnswer({ state: 'found', id: '1', name: 'A', box_art_url: 'u', viewer_count: null })).toEqual({
      state: 'found',
      id: '1',
      name: 'A',
      box_art_url: 'u',
      viewer_count: null,
    });
    expect(readCategoryAnswer({ state: 'found', id: '1', name: 'A', box_art_url: 'u' })?.state).toBe('found');
    expect(readCategoryAnswer({ state: 'missing' })).toEqual({ state: 'missing' });
    expect(readCategoryAnswer({ state: 'unreachable' })).toEqual({ state: 'unreachable' });
  });

  // Catches a found answer without a name being stored as a name: the header
  // would stay a skeleton for good.
  it('rejects an unreadable answer', () => {
    for (const bad of [
      null,
      undefined,
      'found',
      {},
      { state: 'nope' },
      { state: 'found' },
      { state: 'found', id: '1', name: '' },
      { state: 'found', id: '', name: 'A' },
      { state: 'found', id: 1, name: 'A' },
    ]) {
      expect(readCategoryAnswer(bad)).toBeNull();
    }
  });
});

function recordingDeps(calls: string[]): InboundLinkDeps {
  return {
    startStream: (key) => void calls.push(`start:${key}`),
    closeLinkSheet: () => void calls.push('closeSheet'),
    openBadge: (setId, version) => void calls.push(`badge:${setId}/${version}`),
    closeAppLayers: () => void calls.push('layers'),
    openCategory: (id) => void calls.push(`category:${id}`),
  };
}

describe('openInboundLink', () => {
  // Catches a watch link that leaves a badge sheet over the new stream: the
  // sheet closes first, then the stream starts.
  it('closes the link sheet before it starts the stream', () => {
    const calls: string[] = [];
    openInboundLink(watch('xqc'), recordingDeps(calls));
    expect(calls).toEqual(['closeSheet', 'start:xqc']);
  });

  // Catches a badge link that also closes layers or the sheet it is about to open.
  it('opens a badge with its set and version and nothing else', () => {
    const calls: string[] = [];
    openInboundLink(badge('clover', '3'), recordingDeps(calls));
    expect(calls).toEqual(['badge:clover/3']);
  });

  // Catches an opener that waits (an await, a resolve) before the screen opens,
  // or that closes layers after opening the category.
  it('closes the app layers, then opens the category, before it returns', () => {
    const calls: string[] = [];
    openInboundLink(category('509658'), recordingDeps(calls));
    expect(calls).toEqual(['layers', 'category:509658']);
  });

  // Catches a category arm that reaches for any dep beyond the five it is
  // given (a resolve step would show up here as an unknown dep).
  it('uses only the five deps it declares', () => {
    const calls: string[] = [];
    const known = recordingDeps(calls);
    const strict = new Proxy(known, {
      get(target, property) {
        if (!(property in target)) throw new Error(`unknown dep ${String(property)}`);
        return target[property as keyof InboundLinkDeps];
      },
    });
    openInboundLink(category('1'), strict);
    openInboundLink(watch('a'), strict);
    openInboundLink(badge('b'), strict);
    expect(calls).toHaveLength(5);
  });

  // Catches an open that leaves an older held link behind, for any kind.
  it('drops a held link first, for every kind', () => {
    for (const link of [watch('xqc'), badge('clover'), category('1')]) {
      holdLink(badge('old'), 5);
      openInboundLink(link, recordingDeps([]));
      expect(takeHeld()).toBeNull();
    }
  });
});

describe('the held link', () => {
  // Catches a hold that keeps two links, or that hands the same one out twice.
  it('holds one link, last wins, and hands it out once', () => {
    clearHeldLink();
    holdLink(badge('a'), 1);
    holdLink(category('2'), 2);
    expect(takeHeld()).toEqual({ link: category('2'), at: 2 });
    expect(takeHeld()).toBeNull();
  });

  // Catches a clear that leaves the link behind.
  it('clears', () => {
    holdLink(watch('x'), 1);
    clearHeldLink();
    expect(takeHeld()).toBeNull();
  });
});

type Opened = { link: InboundLinkTarget; at: number; mount: boolean };

function harness(over: Partial<Parameters<typeof startLinkDrain>[0]> = {}) {
  let handler: (payload: unknown) => void = () => {};
  let resolveListen: (unlisten: () => void) => void = () => {};
  let rejectListen: (error: unknown) => void = () => {};
  const unlisten = vi.fn();
  const slot: { value: unknown } = { value: null };
  const opened: Opened[] = [];
  const listen = vi.fn((event: string, h: (payload: unknown) => void) => {
    handler = h;
    void event;
    return new Promise<() => void>((res, rej) => {
      resolveListen = res;
      rejectListen = rej;
    });
  });
  const take = vi.fn(async () => {
    const value = slot.value;
    slot.value = null;
    return value;
  });
  const open = vi.fn((link: InboundLinkTarget, info: { at: number; mount: boolean }) => {
    opened.push({ link, ...info });
  });
  const drain = startLinkDrain({ listen, take, open, ...over });
  return {
    drain,
    listen,
    take,
    open,
    opened,
    slot,
    unlisten,
    fire: (payload: unknown) => handler(payload),
    settleListen: () => resolveListen(unlisten),
    failListen: () => rejectListen(new Error('no listen')),
  };
}

describe('startLinkDrain', () => {
  // Catches a mount take that runs before the listener exists: a link stored in
  // the gap would be emitted to nobody and then taken twice or never.
  it('takes on mount only after listen settles, once', async () => {
    const h = harness();
    h.slot.value = taken(watch('early'), 7);
    await flush();
    expect(h.listen).toHaveBeenCalledWith(INBOUND_LINK_EVENT, expect.any(Function));
    expect(h.take).not.toHaveBeenCalled();
    h.settleListen();
    await flush();
    expect(h.take).toHaveBeenCalledTimes(1);
    expect(h.opened).toEqual([{ link: watch('early'), at: 7, mount: true }]);
    h.fire(1);
    await flush();
    expect(h.opened).toHaveLength(1);
  });

  // Catches an event take that carries the mount flag, or that drops the time.
  it('opens an event take with its time and mount false, and a later drain too', async () => {
    const h = harness();
    h.settleListen();
    await flush();
    h.slot.value = taken(badge('clover', '2'), 40);
    h.fire(3);
    await flush();
    h.slot.value = taken(category('9'), 50);
    h.drain.drain();
    await flush();
    expect(h.opened).toEqual([
      { link: badge('clover', '2'), at: 40, mount: false },
      { link: category('9'), at: 50, mount: false },
    ]);
  });

  // Catches a cleanup before listen resolves that still takes or leaks the
  // listener.
  it('a cleanup before listen resolves takes nothing and unlistens when it does', async () => {
    const h = harness();
    h.slot.value = taken(watch('a'));
    h.drain.stop();
    h.settleListen();
    await flush();
    expect(h.take).not.toHaveBeenCalled();
    expect(h.unlisten).toHaveBeenCalledTimes(1);
  });

  // Catches an event that takes after the cleanup.
  it('an event after the cleanup takes nothing', async () => {
    const h = harness();
    h.settleListen();
    await flush();
    h.drain.stop();
    h.slot.value = taken(watch('a'));
    h.fire(4);
    h.drain.drain();
    await flush();
    expect(h.take).toHaveBeenCalledTimes(1);
    expect(h.opened).toHaveLength(0);
    expect(h.unlisten).toHaveBeenCalledTimes(1);
  });

  // Catches a cleanup that throws away a take already in flight: the take
  // emptied the slot, so dropping its link would lose it.
  it('a take already in flight at the cleanup still opens its link', async () => {
    let finish: (value: unknown) => void = () => {};
    const h = harness({
      take: vi.fn(() => new Promise<unknown>((res) => (finish = res))),
    });
    h.settleListen();
    await flush();
    // The mount take is the one in flight now.
    h.drain.stop();
    finish(taken(watch('inflight'), 3));
    await flush();
    expect(h.opened).toEqual([{ link: watch('inflight'), at: 3, mount: true }]);
  });

  // Catches a drain that lets a failing take, an unreadable payload or a bare
  // link (no time) throw or open something.
  it('opens nothing and throws nothing for a rejected take or an unreadable payload', async () => {
    const rejecting = harness({ take: vi.fn(() => Promise.reject(new Error('no command'))) });
    rejecting.settleListen();
    await flush();
    expect(rejecting.opened).toHaveLength(0);

    for (const payload of [null, 'x', { link: watch('a') }, watch('a'), { link: { kind: 'x' }, at: 1 }]) {
      const h = harness();
      h.slot.value = payload;
      h.settleListen();
      await flush();
      expect(h.opened).toHaveLength(0);
    }
  });

  // Catches an open that throws taking the drain down for later links.
  it('a throwing open does not break the next take', async () => {
    const open = vi.fn((link: InboundLinkTarget) => {
      if (link.kind === 'watch' && link.key === 'boom') throw new Error('boom');
    });
    const h = harness({ open });
    h.settleListen();
    await flush();
    h.slot.value = taken(watch('boom'));
    h.fire(1);
    await flush();
    h.slot.value = taken(watch('fine'));
    h.fire(2);
    await flush();
    expect(open).toHaveBeenCalledTimes(2);
  });

  // Catches a failed registration that loses the link stored before it: the
  // mount take still runs once.
  it('a rejected listen still drains once on mount', async () => {
    const h = harness();
    h.slot.value = taken(watch('stored'), 2);
    h.failListen();
    await flush();
    expect(h.take).toHaveBeenCalledTimes(1);
    expect(h.opened).toEqual([{ link: watch('stored'), at: 2, mount: true }]);
  });

  // Catches a rejected listen that drains after the cleanup.
  it('a cleanup before the rejection drains nothing', async () => {
    const h = harness();
    h.slot.value = taken(watch('stored'));
    h.drain.stop();
    h.failListen();
    await flush();
    expect(h.take).not.toHaveBeenCalled();
  });

  // Catches stop forgetting to unlisten, or unlistening before listen settles
  // and never again.
  it('stop calls the unlisten the promise resolved to', async () => {
    const h = harness();
    h.settleListen();
    await flush();
    h.drain.stop();
    await flush();
    expect(h.unlisten).toHaveBeenCalledTimes(1);
  });

  // Catches a stamp that never reaches the boot watch: Tauri hands a listener
  // the whole event, and a test or a peek hands the bare number.
  it('an event with a positive number or a Tauri event reaches onStamp before its take', async () => {
    const order: string[] = [];
    const onStamp = vi.fn((n: number) => void order.push(`stamp:${n}`));
    const h = harness({
      onStamp,
      take: vi.fn(async () => {
        order.push('take');
        return null;
      }),
    });
    h.settleListen();
    await flush();
    order.length = 0;
    h.fire(5);
    h.fire({ event: INBOUND_LINK_EVENT, id: 1, payload: 6 });
    await flush();
    expect(order.slice(0, 2)).toEqual(['stamp:5', 'stamp:6']);
    expect(onStamp.mock.calls.map((c) => c[0])).toEqual([5, 6]);
  });

  // Catches a stamp reader that accepts zero, negatives and non-numbers.
  it('a payload that is not a positive number reaches nothing', async () => {
    const onStamp = vi.fn();
    const h = harness({ onStamp });
    h.settleListen();
    await flush();
    for (const payload of [0, -3, 'x', null, undefined, {}, { payload: 'x' }, { payload: 0 }]) h.fire(payload);
    await flush();
    expect(onStamp).not.toHaveBeenCalled();
  });

  // Catches a peek that runs too late (an action right after mount sees no
  // stamp) or runs when the page is past boot.
  it('peeks at mount and once more after listen while not ready', async () => {
    const onStamp = vi.fn();
    const peek = vi.fn(async () => 3);
    const h = harness({ ready: () => false, peek, onStamp });
    expect(peek).toHaveBeenCalledTimes(1);
    h.settleListen();
    await flush();
    expect(peek).toHaveBeenCalledTimes(2);
    expect(onStamp).toHaveBeenCalledTimes(2);
    expect(onStamp).toHaveBeenCalledWith(3);
    expect(h.take).not.toHaveBeenCalled();
  });

  // Catches a peek that runs while ready, where the link is taken anyway.
  it('never peeks when ready is true or absent', async () => {
    const peek = vi.fn(async () => 3);
    const readyTrue = harness({ ready: () => true, peek });
    readyTrue.settleListen();
    await flush();
    const noReady = harness({ peek });
    noReady.settleListen();
    await flush();
    expect(peek).not.toHaveBeenCalled();
  });

  // Catches a failing peek that throws into the mount.
  it('a rejecting or throwing peek is swallowed', async () => {
    const rejecting = harness({ ready: () => false, peek: vi.fn(() => Promise.reject(new Error('x'))) });
    rejecting.settleListen();
    const throwing = harness({
      ready: () => false,
      peek: vi.fn(() => {
        throw new Error('x');
      }),
    });
    throwing.settleListen();
    await flush();
    expect(rejecting.take).not.toHaveBeenCalled();
  });

  // Catches a drain that takes during boot: the link would leave Rust's slot,
  // where a notice click orders against it, and wait in the page instead.
  it('takes nothing while not ready, then once when ready', async () => {
    let ready = false;
    const h = harness({ ready: () => ready });
    h.settleListen();
    await flush();
    h.slot.value = taken(watch('boot'), 9);
    h.fire(1);
    h.drain.drain();
    await flush();
    expect(h.take).not.toHaveBeenCalled();
    expect(h.slot.value).not.toBeNull();
    ready = true;
    h.drain.drain();
    await flush();
    expect(h.opened).toEqual([{ link: watch('boot'), at: 9, mount: false }]);
    h.drain.drain();
    await flush();
    expect(h.opened).toHaveLength(1);
  });

  // Catches a post-listen mount take that runs while not ready and loses its link.
  it('the post-listen take while not ready takes nothing and loses nothing', async () => {
    const h = harness({ ready: () => false });
    h.slot.value = taken(watch('boot'));
    h.settleListen();
    await flush();
    expect(h.take).not.toHaveBeenCalled();
    expect(h.slot.value).not.toBeNull();
  });

  // Catches two links stored during boot both opening: the slot replaces, so
  // only the newer one is there to take.
  it('only the newer of two links stored during boot opens', async () => {
    let ready = false;
    const h = harness({ ready: () => ready });
    h.settleListen();
    await flush();
    h.slot.value = taken(watch('first'), 1);
    h.fire(1);
    h.slot.value = taken(watch('second'), 2);
    h.fire(2);
    ready = true;
    h.drain.drain();
    await flush();
    expect(h.opened.map((o) => o.link)).toEqual([watch('second')]);
  });

  // Catches a take that does not wait for notice work: the older link would
  // open before the report that supersedes it.
  it('awaits beforeTake before it takes, and ignores its rejection', async () => {
    const order: string[] = [];
    let release: () => void = () => {};
    const beforeTake = vi.fn(
      () =>
        new Promise<void>((res) => {
          release = () => {
            order.push('before');
            res();
          };
        }),
    );
    const h = harness({
      beforeTake,
      take: vi.fn(async () => {
        order.push('take');
        return null;
      }),
    });
    h.settleListen();
    await flush();
    expect(order).toEqual([]);
    release();
    await flush();
    expect(order).toEqual(['before', 'take']);

    const failing = harness({ beforeTake: () => Promise.reject(new Error('x')) });
    failing.slot.value = taken(watch('still'));
    failing.settleListen();
    await flush();
    expect(failing.opened).toHaveLength(1);
  });

  // Catches a cleanup during beforeTake that still takes.
  it('a cleanup while beforeTake is pending takes nothing', async () => {
    let release: () => void = () => {};
    const h = harness({ beforeTake: () => new Promise<void>((res) => (release = res)) });
    h.settleListen();
    await flush();
    h.drain.stop();
    release();
    await flush();
    expect(h.take).not.toHaveBeenCalled();
  });

  // Catches a drain handed to an event listener that treats the event as the
  // mount flag.
  it('drain ignores the arguments it is called with', async () => {
    const h = harness();
    h.settleListen();
    await flush();
    h.slot.value = taken(watch('x'));
    (h.drain.drain as (arg?: unknown) => void)({ type: 'visibilitychange' });
    await flush();
    expect(h.opened[0].mount).toBe(false);
  });
});

describe('startPhoneLinkBridge', () => {
  function bridge(tapped: boolean) {
    let handler: (payload: unknown) => void = () => {};
    let settle: (unlisten: () => void) => void = () => {};
    const slot: { value: unknown } = { value: null };
    const opened: InboundLinkTarget[] = [];
    const take = vi.fn(async () => {
      const value = slot.value;
      slot.value = null;
      return value;
    });
    const unlisten = vi.fn();
    const handle = startPhoneLinkBridge({
      listen: (_event, h) => {
        handler = h;
        return new Promise<() => void>((res) => (settle = res));
      },
      take,
      tapped,
      open: (link) => void opened.push(link),
    });
    return { handle, take, slot, opened, unlisten, fire: (n: unknown) => handler(n), settle: () => settle(unlisten) };
  }

  // Catches a bridge that opens the stored watch link on a mount that consumed
  // a tap: the tap is the newer action.
  it('a tapped mount drops the watch link its own take returns', async () => {
    const b = bridge(true);
    b.slot.value = taken(watch('stale'));
    b.settle();
    await flush();
    expect(b.take).toHaveBeenCalledTimes(1);
    expect(b.opened).toEqual([]);
  });

  // Catches a bridge that drops every kind on a tapped mount.
  it('a tapped mount still opens a badge link and a category link', async () => {
    const badgeBridge = bridge(true);
    badgeBridge.slot.value = taken(badge('clover', '3'));
    badgeBridge.settle();
    await flush();
    expect(badgeBridge.opened).toEqual([badge('clover', '3')]);

    const categoryBridge = bridge(true);
    categoryBridge.slot.value = taken(category('509658'));
    categoryBridge.settle();
    await flush();
    expect(categoryBridge.opened).toEqual([category('509658')]);
  });

  // Catches a bridge that drops a watch link from a later event: only the
  // mount's own take is told apart.
  it('a tapped mount opens a watch link that arrives by event later', async () => {
    const b = bridge(true);
    b.settle();
    await flush();
    b.slot.value = taken(watch('later'));
    b.fire(2);
    await flush();
    expect(b.opened).toEqual([watch('later')]);
  });

  // Catches "the first take" standing in for the mount take: an event can take
  // before listen settles, and its link is newer than the tap.
  it('an event take before listen settles opens, the listen-settle take after it drops', async () => {
    const b = bridge(true);
    b.slot.value = taken(watch('event'));
    b.fire(1);
    await flush();
    expect(b.opened).toEqual([watch('event')]);
    b.slot.value = taken(watch('stored'));
    b.settle();
    await flush();
    expect(b.opened).toEqual([watch('event')]);
  });

  // Catches a bridge that drops the mount's watch link on an untapped mount.
  it('an untapped mount opens the first take watch link', async () => {
    const b = bridge(false);
    b.slot.value = taken(watch('stored'));
    b.settle();
    await flush();
    expect(b.opened).toEqual([watch('stored')]);
  });

  // Catches a bridge that waits for boot or peeks: the phone has no boot rule.
  it('takes right after listen settles with no ready rule', async () => {
    const b = bridge(false);
    b.settle();
    await flush();
    expect(b.take).toHaveBeenCalledTimes(1);
  });

  // Catches a stop that leaves the listener registered.
  it('stop unlistens', async () => {
    const b = bridge(false);
    b.settle();
    await flush();
    b.handle.stop();
    await flush();
    expect(b.unlisten).toHaveBeenCalledTimes(1);
  });
});

describe('inboundLinkOpen source', () => {
  // Catches a second spelling of the event name: the module owns the one.
  it('spells the event name once', () => {
    expect(source.split("'streamnook:link'")).toHaveLength(2);
  });

  // Catches the bridge's options type loosening `tapped` to a string: the phone
  // shell folds its three consumed taps to one boolean at the call.
  it('declares the bridge option tapped as a boolean', () => {
    expect(source).toContain('  tapped: boolean;');
  });

  // Catches a resolve step coming back into the opener: the screen resolves its
  // own id, and the opener never calls into Rust.
  it('calls no command and names no resolve', () => {
    expect(source).not.toMatch(/\binvoke\b/);
    expect(source).not.toContain('resolve_' + 'category');
  });
});

describe('createPhoneLinkDoors', () => {
  const label = (link: InboundLinkTarget) =>
    link.kind === 'watch' ? `watch:${link.key}` : link.kind === 'badge' ? `badge:${link.setId}` : `category:${link.id}`;

  function doorsWith(opts: { hold?: boolean; now?: number } = {}) {
    const state = { hold: opts.hold ?? false, now: opts.now ?? 0 };
    const opened: string[] = [];
    const deps = {
      shouldHold: vi.fn(() => state.hold),
      openMention: vi.fn((id: string) => void opened.push(`mention:${id}`)),
      openWhisper: vi.fn((key: string) => void opened.push(`whisper:${key}`)),
      openChannel: vi.fn((login: string) => void opened.push(`channel:${login}`)),
      onLink: vi.fn((link: InboundLinkTarget) => void opened.push(label(link))),
      bootNow: vi.fn(() => state.now),
    };
    clearHeldLink();
    logs.info.mockClear();
    return { doors: createPhoneLinkDoors(deps), deps, opened, state };
  }

  it('orders by known times only', () => {
    expect(isOlder(100, 150)).toBe(true);
    expect(isOlder(150, 100)).toBe(false);
    expect(isOlder(150, 150)).toBe(false);
    expect(isOlder(0, 150)).toBe(false);
    expect(isOlder(100, 0)).toBe(false);
  });

  // Fails if an older link that the page hears late opens over a newer tap.
  it('a link at 100, a tap at 150: the tap opens, and a later take of a link from 100 is dropped', () => {
    const d = doorsWith();
    d.doors.onLink(category('1'), 100);
    d.doors.channelTap('xqc', 150);
    d.doors.onLink(category('2'), 100);
    expect(d.opened).toEqual(['category:1', 'channel:xqc']);
    expect(logs.info).toHaveBeenCalledWith('[link] dropped, a tap is newer');
  });

  it('a tap at 100, a link at 150: both open, the link last', () => {
    const d = doorsWith();
    d.doors.channelTap('xqc', 100);
    d.doors.onLink(category('1'), 150);
    expect(d.opened).toEqual(['channel:xqc', 'category:1']);
  });

  // Fails if a tap the page hears late opens over a newer link.
  it('a tap older than the newest link opens nothing and keeps a held link', () => {
    const d = doorsWith({ hold: true });
    d.doors.onLink(category('1'), 200);
    d.doors.channelTap('xqc', 150);
    expect(d.opened).toEqual([]);
    expect(heldAt()).toBe(200);
    expect(logs.info).toHaveBeenCalledWith('[link] tap skipped, a link is newer');
  });

  it('a newer tap clears an older held link and opens; release then opens nothing', () => {
    const d = doorsWith({ hold: true });
    d.doors.onLink(category('1'), 100);
    d.doors.mentionTap('m1', 150);
    expect(d.opened).toEqual(['mention:m1']);
    expect(heldAt()).toBeNull();
    d.state.hold = false;
    d.doors.release();
    expect(d.opened).toEqual(['mention:m1']);
  });

  it('a tap or a link with no time opens and orders nothing', () => {
    const d = doorsWith({ hold: true });
    d.doors.onLink(category('1'), 200);
    d.doors.whisperTap('w1', 0);
    expect(d.opened).toEqual(['whisper:w1']);
    expect(heldAt()).toBeNull();
    const e = doorsWith();
    e.doors.channelTap('xqc', 150);
    e.doors.onLink(category('2'), 0);
    expect(e.opened).toEqual(['channel:xqc', 'category:2']);
  });

  it('an empty tap does nothing and keeps the held link', () => {
    const d = doorsWith({ hold: true });
    d.doors.onLink(badge('clover', '3'), 100);
    d.doors.channelTap('', 150);
    d.doors.mentionTap('', 150);
    d.doors.whisperTap('', 150);
    expect(d.opened).toEqual([]);
    expect(heldAt()).toBe(100);
  });

  // Fails if a watch link waits behind picture in picture, or asks whether to.
  it('a watch link never waits', () => {
    const d = doorsWith({ hold: true });
    d.doors.onLink(watch('xqc'), 100);
    expect(d.opened).toEqual(['watch:xqc']);
    expect(d.deps.shouldHold).not.toHaveBeenCalled();
  });

  // Fails if a badge or category link opens behind a window the person is not looking at.
  it('a badge or category link waits while held; the newer replaces the older; release opens it once', () => {
    const d = doorsWith({ hold: true });
    d.doors.onLink(badge('clover', '3'), 100);
    d.doors.onLink(category('1'), 120);
    expect(d.opened).toEqual([]);
    d.doors.release();
    expect(d.opened).toEqual([]);
    expect(heldAt()).toBe(120);
    d.state.hold = false;
    d.doors.release();
    d.doors.release();
    expect(d.opened).toEqual(['category:1']);
  });

  // Fails without the minute limit, or if a 0 time ever drops a held link.
  it('a link held more than a minute is dropped; a 0 on either side never drops', () => {
    const late = doorsWith({ hold: true });
    late.doors.onLink(category('1'), 1000);
    late.state.hold = false;
    late.state.now = 61_001;
    late.doors.release();
    expect(late.opened).toEqual([]);
    const inTime = doorsWith({ hold: true });
    inTime.doors.onLink(category('1'), 1000);
    inTime.state.hold = false;
    inTime.state.now = 60_999;
    inTime.doors.release();
    expect(inTime.opened).toEqual(['category:1']);
    const noClock = doorsWith({ hold: true });
    noClock.doors.onLink(category('1'), 1000);
    noClock.state.hold = false;
    noClock.doors.release();
    expect(noClock.opened).toEqual(['category:1']);
    const noTime = doorsWith({ hold: true, now: 999_999 });
    noTime.doors.onLink(category('1'), 0);
    noTime.state.hold = false;
    noTime.doors.release();
    expect(noTime.opened).toEqual(['category:1']);
  });

  it('a mount tap runs through its own door', () => {
    const d = doorsWith();
    d.doors.mountTap({ kind: 'whisper', value: 'w1', at: 5 });
    d.doors.mountTap({ kind: 'mention', value: 'm1', at: 6 });
    d.doors.mountTap({ kind: 'channel', value: 'xqc', at: 7 });
    d.doors.mountTap(null);
    expect(d.opened).toEqual(['whisper:w1', 'mention:m1', 'channel:xqc']);
    expect(d.deps.openWhisper).toHaveBeenCalledWith('w1');
    expect(d.deps.openMention).toHaveBeenCalledWith('m1');
    expect(d.deps.openChannel).toHaveBeenCalledWith('xqc');
  });

  // Fails if the mount's own take opens a link older than the tap the mount ran.
  it('a mount with a tap at 150 drops a slot link from 100 and opens one from 200 after the tap', async () => {
    for (const [at, expected] of [
      [100, ['channel:xqc']],
      [200, ['channel:xqc', 'watch:new']],
    ] as const) {
      const d = doorsWith();
      const stop = d.doors.mountDrain({
        tap: { kind: 'channel', value: 'xqc', at: 150 },
        listen: async () => () => {},
        take: async () => taken(watch('new'), at),
      });
      await flush();
      expect(d.opened).toEqual(expected);
      stop();
    }
  });

  // Fails if an event take that lands while another take is in flight reorders the end state.
  it('an event take of a link older than a tap heard first is dropped', async () => {
    const d = doorsWith();
    let fire: (payload: unknown) => void = () => {};
    const slot: { value: unknown } = { value: null };
    const stop = d.doors.mountDrain({
      tap: null,
      listen: async (_event, handler) => {
        fire = handler;
        return () => {};
      },
      take: async () => {
        const value = slot.value;
        slot.value = null;
        return value;
      },
    });
    await flush();
    d.doors.channelTap('xqc', 2000);
    slot.value = taken(watch('stale'), 1500);
    fire(1);
    await flush();
    slot.value = taken(watch('fresh'), 2500);
    fire(2);
    await flush();
    expect(d.opened).toEqual(['channel:xqc', 'watch:fresh']);
    stop();
  });

  it('with no tap the mount opens the stored link once', async () => {
    const d = doorsWith();
    const stop = d.doors.mountDrain({ tap: null, listen: async () => () => {}, take: async () => taken(badge('clover', '3'), 9) });
    await flush();
    expect(d.opened).toEqual(['badge:clover']);
    stop();
  });
});
