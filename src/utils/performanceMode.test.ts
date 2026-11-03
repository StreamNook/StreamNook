// Performance mode in a window: it listens before it asks, an event that lands
// first wins over a stale read, a failed read leaves the full look, a settings
// seed covers the first paint until Rust answers, and the first paint never
// waits more than its cap.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

type State = { active: boolean; still_chat: boolean };
type Handler = (e: { payload: State }) => void;

const h = vi.hoisted(() => {
  const deferred = <T>() => {
    let resolve!: (v: T) => void;
    let reject!: (e: unknown) => void;
    const promise = new Promise<T>((res, rej) => {
      resolve = res;
      reject = rej;
    });
    return { promise, resolve, reject };
  };
  return {
    deferred,
    order: [] as string[],
    handler: null as Handler | null,
    listen: null as null | { promise: Promise<() => void>; resolve: (v: () => void) => void; reject: (e: unknown) => void },
    invoke: null as null | { promise: Promise<State>; resolve: (v: State) => void; reject: (e: unknown) => void },
    peek: null as null | { promise: Promise<unknown>; resolve: (v: unknown) => void; reject: (e: unknown) => void },
    flat: [] as boolean[],
  };
});

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string) => {
    h.order.push(`invoke:${cmd}`);
    return h.invoke!.promise;
  },
}));
vi.mock('@tauri-apps/api/event', () => ({
  listen: (name: string, cb: Handler) => {
    h.order.push(`listen:${name}`);
    h.handler = cb;
    return h.listen!.promise;
  },
}));
vi.mock('../themes', () => ({ setPerformanceFlat: (on: boolean) => h.flat.push(on) }));
vi.mock('../bootPreload', () => ({ peekPreloadedSettings: () => h.peek?.promise ?? null }));
vi.mock('../stores/AppStore', () => ({
  useAppStore: { getState: () => ({ settings: {} }), subscribe: () => () => {} },
}));
vi.mock('./logger', () => ({ Logger: { debug: () => {} } }));

const flush = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};

async function load() {
  vi.resetModules();
  const pm = await import('./performanceMode');
  const lp = await import('./lowPower');
  return { pm, lp };
}

beforeEach(() => {
  h.order = [];
  h.handler = null;
  h.flat = [];
  h.listen = h.deferred<() => void>();
  h.invoke = h.deferred<State>();
  h.peek = null;
});
afterEach(() => {
  vi.useRealTimers();
});

describe('installPerformanceMode', () => {
  it('asks Rust only after its listener is registered', async () => {
    const { pm } = await load();
    void pm.installPerformanceMode();
    await flush();
    expect(h.order).toEqual(['listen:performance-state']);
    h.listen!.resolve(() => {});
    await flush();
    expect(h.order).toEqual(['listen:performance-state', 'invoke:get_performance_state']);
  });

  it('an event that lands before the read keeps its state', async () => {
    const { pm, lp } = await load();
    const done = pm.installPerformanceMode();
    h.listen!.resolve(() => {});
    await flush();
    h.handler!({ payload: { active: true, still_chat: false } });
    h.invoke!.resolve({ active: false, still_chat: false });
    await done;
    expect(lp.isLowPower()).toBe(true);
    expect(lp.isFlat()).toBe(true);
    expect(h.flat).toEqual([true]);
  });

  it('applies the read when no event came, even with a late listener', async () => {
    const { pm, lp } = await load();
    const done = pm.installPerformanceMode();
    await new Promise((r) => setTimeout(r, 20));
    h.listen!.resolve(() => {});
    await flush();
    h.invoke!.resolve({ active: true, still_chat: true });
    await done;
    expect(lp.isLowPower()).toBe(true);
    expect(lp.isStillChat()).toBe(true);
  });

  it('a failed read leaves the full look', async () => {
    const { pm, lp } = await load();
    const done = pm.installPerformanceMode();
    h.listen!.resolve(() => {});
    await flush();
    h.invoke!.reject(new Error('denied'));
    await done;
    expect(lp.isLowPower()).toBe(false);
    expect(h.flat).toEqual([]);
  });

  it('a failed listen still reads the state', async () => {
    const { pm, lp } = await load();
    const done = pm.installPerformanceMode();
    h.listen!.reject(new Error('no events'));
    await flush();
    h.invoke!.resolve({ active: true, still_chat: false });
    await done;
    expect(lp.isLowPower()).toBe(true);
  });

  it('seeds Always from the preloaded settings until Rust answers', async () => {
    h.peek = h.deferred<unknown>();
    const { pm, lp } = await load();
    const done = pm.installPerformanceMode();
    h.peek.resolve({ performance_mode: 'always', performance_still_chat: true });
    await flush();
    expect(lp.isLowPower()).toBe(true);
    expect(lp.isStillChat()).toBe(true);
    h.listen!.resolve(() => {});
    await flush();
    h.invoke!.resolve({ active: false, still_chat: false });
    await done;
    expect(lp.isLowPower()).toBe(false);
    expect(lp.isStillChat()).toBe(false);
  });

  it('a seed that arrives after Rust answered is ignored', async () => {
    h.peek = h.deferred<unknown>();
    const { pm, lp } = await load();
    const done = pm.installPerformanceMode();
    h.listen!.resolve(() => {});
    await flush();
    h.invoke!.resolve({ active: false, still_chat: false });
    await done;
    h.peek.resolve({ performance_mode: 'always', performance_still_chat: false });
    await flush();
    expect(lp.isLowPower()).toBe(false);
  });

  it('Background in the settings seeds nothing', async () => {
    h.peek = h.deferred<unknown>();
    const { pm, lp } = await load();
    void pm.installPerformanceMode();
    h.peek.resolve({ performance_mode: 'background' });
    await flush();
    expect(lp.isLowPower()).toBe(false);
  });

  it('is installed once per window', async () => {
    const { pm } = await load();
    const a = pm.installPerformanceMode();
    const b = pm.installPerformanceMode();
    expect(a).toBe(b);
    await flush();
    expect(h.order.filter((o) => o.startsWith('listen:'))).toHaveLength(1);
  });
});

describe('paintAfter', () => {
  it('resolves when the promise settles', async () => {
    const { pm } = await load();
    let done = false;
    void pm.paintAfter(Promise.resolve(), 300).then(() => (done = true));
    await flush();
    expect(done).toBe(true);
  });

  it('resolves after the cap when the promise hangs', async () => {
    vi.useFakeTimers();
    const { pm } = await load();
    let done = false;
    void pm.paintAfter(new Promise(() => {}), 300).then(() => (done = true));
    await vi.advanceTimersByTimeAsync(299);
    expect(done).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    expect(done).toBe(true);
  });

  it('never rejects', async () => {
    const { pm } = await load();
    await expect(pm.paintAfter(Promise.reject(new Error('x')), 300)).resolves.toBeUndefined();
  });
});

describe('leaving the cheap look', () => {
  const g = globalThis as Record<string, unknown>;
  afterEach(() => {
    delete g.document;
    delete g.CSSAnimation;
  });

  it('applies the restored timing, then finishes only finite, non-exempt animations started before it', async () => {
    const log: string[] = [];
    const { CHEAP_EXEMPT } = await import('./cheapExempt');
    class FakeCSSAnimation {
      constructor(
        readonly id: string,
        readonly startTime: number | null,
        private readonly iterations: number,
        private readonly exempt: boolean,
      ) {}
      get playState() {
        log.push(`playState:${this.id}`);
        return 'running';
      }
      get effect() {
        return {
          getComputedTiming: () => ({ iterations: this.iterations }),
          target: { matches: (sel: string) => sel === CHEAP_EXEMPT && this.exempt },
        };
      }
      finish() {
        log.push(`finish:${this.id}`);
      }
    }
    const anims = [
      new FakeCSSAnimation('entrance', 100, 1, false),
      new FakeCSSAnimation('loop', 100, Infinity, false),
      new FakeCSSAnimation('cue', 100, 1, true),
      new FakeCSSAnimation('after', 900, 1, false),
      new FakeCSSAnimation('pending', null, 1, false),
    ];
    g.CSSAnimation = FakeCSSAnimation;
    g.document = {
      timeline: { currentTime: 500 },
      getAnimations: () => anims,
      documentElement: { dataset: {} },
    };
    const { pm } = await load();
    pm.applyPerformanceState({ active: true, still_chat: false });
    expect(log).toEqual([]);
    pm.applyPerformanceState({ active: false, still_chat: false });
    expect(log).toEqual(['playState:entrance', 'finish:entrance']);
  });
});
