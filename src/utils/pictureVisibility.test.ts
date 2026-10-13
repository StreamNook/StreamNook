// Run with: npm test
//
// Whether a moving profile picture is seen, driven end to end through the
// shared watcher with a fake document: the observer reports, the covering
// layers come and go through the events that announce them, and the picture
// hears in view / out of view.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const store = vi.hoisted(() => ({
  listeners: new Set<(next: Record<string, unknown>, prev: Record<string, unknown>) => void>(),
  state: { isSettingsOpen: false, isPlayerFullscreen: false } as Record<string, unknown>,
}));

vi.mock('../stores/AppStore', () => ({
  useAppStore: {
    subscribe: (listener: (next: Record<string, unknown>, prev: Record<string, unknown>) => void) => {
      store.listeners.add(listener);
      return () => store.listeners.delete(listener);
    },
  },
}));

vi.mock('./windowVisibility', () => ({
  isWindowHidden: () => false,
  onWindowVisibility: () => () => {},
}));

import { COVERED_CONTAINER_SELECTOR } from './coverage';
import { pictureInView, watchPictureVisibility } from './pictureVisibility';

class FakeEl {
  parent: FakeEl | null;
  opacity = 1;
  visibility = 'visible';
  coveredMark = false;
  layer = false;
  isConnected = true;
  constructor(
    readonly name: string,
    parent: FakeEl | null = null,
  ) {
    this.parent = parent;
  }
  contains(other: FakeEl | null): boolean {
    for (let n = other; n; n = n.parent) if (n === this) return true;
    return false;
  }
  *ancestry(): Generator<FakeEl> {
    yield this;
    for (let n = this.parent; n; n = n.parent) yield n;
  }
  closest(selector: string): FakeEl | null {
    if (selector !== COVERED_CONTAINER_SELECTOR) return null;
    for (const n of this.ancestry()) if (n.coveredMark) return n;
    return null;
  }
  checkVisibility(): boolean {
    for (const n of this.ancestry()) {
      if (n.opacity === 0 || n.visibility === 'hidden') return false;
    }
    return true;
  }
  matches(): boolean {
    return this.layer;
  }
  getBoundingClientRect() {
    return { left: 10, right: 42, top: 10, bottom: 42 };
  }
}

const docListeners = new Map<string, Set<(e: unknown) => void>>();
let layers: FakeEl[] = [];
let stack: FakeEl[] = [];
let observerCallback: ((entries: unknown[]) => void) | null = null;
const observed = new Set<unknown>();

function fire(type: string, event: unknown = {}) {
  docListeners.get(type)?.forEach((listener) => listener(event));
}

function report(el: FakeEl, inView: boolean, area = 32) {
  observerCallback?.([
    {
      target: el,
      isIntersecting: inView,
      intersectionRect: { width: inView ? area : 0, height: inView ? area : 0 },
    },
  ]);
}

function setStore(patch: Record<string, unknown>) {
  const prev = store.state;
  store.state = { ...prev, ...patch };
  store.listeners.forEach((listener) => listener(store.state, prev));
}

const root = new FakeEl('root');
const sidebar = new FakeEl('sidebar', root);
const avatar = new FakeEl('avatar', sidebar);
const overlay = new FakeEl('settings overlay', root);
overlay.layer = true;

beforeEach(() => {
  vi.useFakeTimers();
  layers = [];
  stack = [avatar, sidebar, root];
  sidebar.opacity = 1;
  root.coveredMark = false;
  overlay.isConnected = true;
  // The observer is created once per module, like in the app, so its
  // callback is the one captured the first time.
  observed.clear();
  docListeners.clear();
  store.listeners.clear();
  vi.stubGlobal(
    'IntersectionObserver',
    class {
      constructor(cb: (entries: unknown[]) => void) {
        observerCallback = cb;
      }
      observe(el: unknown) {
        observed.add(el);
      }
      unobserve(el: unknown) {
        observed.delete(el);
      }
    },
  );
  vi.stubGlobal('document', {
    fullscreenElement: null,
    addEventListener: (type: string, listener: (e: unknown) => void) => {
      if (!docListeners.has(type)) docListeners.set(type, new Set());
      docListeners.get(type)!.add(listener);
    },
    removeEventListener: (type: string, listener: (e: unknown) => void) => {
      docListeners.get(type)?.delete(listener);
    },
    querySelectorAll: () => layers.filter((layer) => layer.isConnected),
    elementsFromPoint: () => stack,
  });
  vi.stubGlobal('window', {
    innerWidth: 1280,
    innerHeight: 720,
    addEventListener: () => {},
    removeEventListener: () => {},
  });
  vi.stubGlobal('Element', FakeEl);
});

const stops: Array<() => void> = [];

afterEach(() => {
  while (stops.length) stops.pop()!();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('pictureInView', () => {
  it('needs on screen, drawn and uncovered all at once', () => {
    expect(pictureInView({ onScreen: true, drawn: true, covered: false })).toBe(true);
    expect(pictureInView({ onScreen: false, drawn: true, covered: false })).toBe(false);
    expect(pictureInView({ onScreen: true, drawn: false, covered: false })).toBe(false);
    expect(pictureInView({ onScreen: true, drawn: true, covered: true })).toBe(false);
  });
});

describe('watchPictureVisibility', () => {
  function watch() {
    const heard: boolean[] = [];
    let stopped = false;
    const unwatch = watchPictureVisibility(avatar as unknown as Element, (v) => heard.push(v));
    const stop = () => {
      if (stopped) return;
      stopped = true;
      unwatch();
    };
    stops.push(stop);
    return { heard, stop };
  }

  it('reports the observer, and treats a picture clipped to nothing as out of view', () => {
    const { heard, stop } = watch();
    report(avatar, true);
    expect(heard).toEqual([true]);
    report(avatar, true, 0);
    expect(heard).toEqual([true, false]);
    stop();
  });

  it('pauses under an overlay opened through the store and resumes when it unmounts', () => {
    const { heard, stop } = watch();
    report(avatar, true);
    layers = [overlay];
    stack = [overlay, avatar, sidebar, root];
    setStore({ isSettingsOpen: true });
    vi.advanceTimersByTime(200);
    expect(heard).toEqual([true, false]);

    // Closed, and gone after its exit animation, with no event of its own.
    overlay.isConnected = false;
    layers = [];
    stack = [avatar, sidebar, root];
    vi.advanceTimersByTime(600);
    expect(heard).toEqual([true, false, true]);
    stop();
  });

  it('pauses under the full-screen player entered by a key', () => {
    const { heard, stop } = watch();
    report(avatar, true);
    const player = new FakeEl('plyr', root);
    player.layer = true;
    layers = [player];
    stack = [player, avatar, sidebar, root];
    fire('keyup');
    vi.advanceTimersByTime(200);
    expect(heard).toEqual([true, false]);
    stop();
  });

  it('pauses in a panel faded out, after its transition ends', () => {
    const { heard, stop } = watch();
    report(avatar, true);
    sidebar.opacity = 0;
    fire('transitionend', { propertyName: 'color', target: sidebar });
    vi.advanceTimersByTime(200);
    expect(heard).toEqual([true]);
    fire('transitionend', { propertyName: 'opacity', target: sidebar });
    vi.advanceTimersByTime(200);
    expect(heard).toEqual([true, false]);
    sidebar.opacity = 1;
    fire('transitionend', { propertyName: 'opacity', target: sidebar });
    vi.advanceTimersByTime(200);
    expect(heard).toEqual([true, false, true]);
    stop();
  });

  it('pauses inside a container the app marks covered', () => {
    const { heard, stop } = watch();
    report(avatar, true);
    root.coveredMark = true;
    fire('pointerup');
    vi.advanceTimersByTime(200);
    expect(heard).toEqual([true, false]);
    stop();
  });

  it('leaves no listener behind once the last picture goes', () => {
    const { stop } = watch();
    expect(docListeners.get('pointerup')?.size).toBe(1);
    expect(store.listeners.size).toBe(1);
    stop();
    expect(docListeners.get('pointerup')?.size).toBe(0);
    expect(store.listeners.size).toBe(0);
    expect(observed.size).toBe(0);
  });
});
