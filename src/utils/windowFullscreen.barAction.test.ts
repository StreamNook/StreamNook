// @vitest-environment node
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// The title bar's full-screen helpers, end to end through their real DOM and
// Tauri seams: which clicks leave the player's full screen, how the exit is
// made, and what a press on the bar does in and out of full screen. The pure
// rules underneath are covered in titleBarTuck.test.ts; this file covers the
// wiring around them.

const h = vi.hoisted(() => {
  const invoke = vi.fn((_cmd: string, _args?: unknown) => Promise.resolve());
  const win = {
    isFullscreen: vi.fn(async () => true),
    isMaximized: vi.fn(async () => false),
    setFullscreen: vi.fn(async (_on: boolean) => {}),
    unmaximize: vi.fn(async () => {}),
    maximize: vi.fn(async () => {}),
    setPosition: vi.fn(async (_p: unknown) => {}),
    toggleMaximize: vi.fn(async () => {}),
  };
  const store = { isWindowFullscreen: false, isPlayerFullscreen: true, playerOverlayVisible: false };
  const listeners: Record<string, (() => void)[]> = {};
  return { invoke, win, store, listeners };
});

vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: unknown) => h.invoke(cmd, args) }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: (name: string, cb: () => void) => {
    (h.listeners[name] ??= []).push(cb);
    return Promise.resolve(() => {});
  },
}));
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => h.win,
  currentMonitor: async () => null,
  PhysicalPosition: class {
    constructor(
      public x: number,
      public y: number,
    ) {}
  },
}));
vi.mock('../stores/AppStore', () => ({
  useAppStore: {
    getState: () => h.store,
    setState: (patch: Partial<typeof h.store>) => Object.assign(h.store, patch),
  },
}));
vi.mock('./logger', () => ({
  Logger: { error: vi.fn(), warn: vi.fn(), info: vi.fn(), debug: vi.fn() },
}));

// A small element double: enough of Element for `closest`, `hasAttribute` and
// `instanceof Element`, with a selector matcher that understands the simple
// selectors the helpers use (`button`, `a[href]`, `[role="x"]`, `[attr]`).
class FakeElement {
  constructor(
    public tag: string,
    public attrs: Record<string, string> = {},
    public parent: FakeElement | null = null,
  ) {}
  hasAttribute(name: string): boolean {
    return name in this.attrs;
  }
  getAttribute(name: string): string | null {
    return this.attrs[name] ?? null;
  }
  matches(selectorList: string): boolean {
    return selectorList.split(',').some((s) => matchOne(this, s.trim()));
  }
  closest(selectorList: string): FakeElement | null {
    if (this.matches(selectorList)) return this;
    return this.parent ? this.parent.closest(selectorList) : null;
  }
  child(tag: string, attrs: Record<string, string> = {}): FakeElement {
    return new FakeElement(tag, attrs, this);
  }
}

function matchOne(el: FakeElement, selector: string): boolean {
  const m = /^([a-z0-9]*)((?:\[[^\]]+\])*)$/i.exec(selector);
  if (!m) throw new Error(`selector not supported by the test double: ${selector}`);
  const [, tag, attrPart] = m;
  if (tag && tag.toLowerCase() !== el.tag) return false;
  for (const [, name, value] of attrPart.matchAll(/\[([^\]=]+)(?:="([^"]*)")?\]/g)) {
    if (!el.hasAttribute(name)) return false;
    if (value !== undefined && el.getAttribute(name) !== value) return false;
  }
  return true;
}

type Listener = (e: { screenX: number; screenY: number }) => void;
const windowListeners = new Map<string, Set<Listener>>();
const g = globalThis as Record<string, unknown>;
g.Element = FakeElement;
g.window = Object.assign(globalThis, {
  addEventListener: (type: string, fn: Listener) => {
    if (!windowListeners.has(type)) windowListeners.set(type, new Set());
    windowListeners.get(type)!.add(fn);
  },
  removeEventListener: (type: string, fn: Listener) => {
    windowListeners.get(type)?.delete(fn);
  },
});
const fire = (type: string, e = { screenX: 0, screenY: 0 }) => {
  for (const fn of [...(windowListeners.get(type) ?? [])]) fn(e);
};

const plyr = { fullscreen: { active: true, exit: vi.fn() } };
let fullscreenMedia: { plyr?: typeof plyr } | null = { plyr };
g.document = {
  documentElement: { dataset: {} as Record<string, string> },
  querySelector: vi.fn(() => fullscreenMedia),
};

const { exitPlayerFullscreen, exitPlayerFullscreenForBarAction: barActionReal } = await import('./windowFullscreen');
const exitPlayerFullscreenForBarAction = (target: FakeElement | EventTarget | null) =>
  barActionReal(target as EventTarget | null);
const { handleTitleBarMouseDown, isTitlebarDragActive } = await import('./titleBarDrag');

/** The bar, built the way TitleBar.tsx lays it out. */
const buildBar = () => {
  const bar = new FakeElement('div', { 'data-titlebar': '' });
  const left = bar.child('div', { 'data-titlebar-left': '' });
  const right = bar.child('div', { 'data-titlebar-right': '' });
  const caption = right.child('div', { 'data-keeps-player-fullscreen': '' });
  return { bar, left, right, caption };
};

const settle = async () => {
  await vi.dynamicImportSettled();
  for (let i = 0; i < 5; i++) await Promise.resolve();
};

beforeEach(() => {
  vi.clearAllMocks();
  plyr.fullscreen.active = true;
  fullscreenMedia = { plyr };
  Object.assign(h.store, { isWindowFullscreen: false, isPlayerFullscreen: true });
  windowListeners.clear();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('exitPlayerFullscreenForBarAction', () => {
  it('leaves through the player for a bar control that changes the page', () => {
    const { left } = buildBar();
    exitPlayerFullscreenForBarAction(left.child('button'));
    expect(plyr.fullscreen.exit).toHaveBeenCalledTimes(1);
  });

  it('resolves an icon inside a button to the button', () => {
    const { left } = buildBar();
    const icon = left.child('button').child('svg').child('path');
    exitPlayerFullscreenForBarAction(icon);
    expect(plyr.fullscreen.exit).toHaveBeenCalledTimes(1);
  });

  it('leaves for an item picked in a menu the bar portals out', () => {
    const menu = new FakeElement('div', { role: 'menu' });
    exitPlayerFullscreenForBarAction(menu.child('button', { role: 'menuitemradio' }).child('span'));
    expect(plyr.fullscreen.exit).toHaveBeenCalledTimes(1);
  });

  it('keeps full screen for a menu trigger (aria-expanded or aria-haspopup)', () => {
    const { left } = buildBar();
    exitPlayerFullscreenForBarAction(left.child('button', { 'aria-expanded': 'false' }));
    exitPlayerFullscreenForBarAction(left.child('button', { 'aria-haspopup': 'menu' }));
    expect(plyr.fullscreen.exit).not.toHaveBeenCalled();
  });

  it('keeps full screen for the caption buttons', () => {
    const { caption } = buildBar();
    exitPlayerFullscreenForBarAction(caption.child('button').child('svg'));
    expect(plyr.fullscreen.exit).not.toHaveBeenCalled();
  });

  it('keeps full screen for a press on empty bar', () => {
    const { bar } = buildBar();
    exitPlayerFullscreenForBarAction(bar);
    expect(plyr.fullscreen.exit).not.toHaveBeenCalled();
  });

  it('ignores a target that is not an element', () => {
    expect(() => exitPlayerFullscreenForBarAction(null)).not.toThrow();
    expect(() => exitPlayerFullscreenForBarAction({} as EventTarget)).not.toThrow();
    expect(plyr.fullscreen.exit).not.toHaveBeenCalled();
  });
});

describe('exitPlayerFullscreen', () => {
  it('goes through the player when one is in full screen, and touches the window no other way', async () => {
    exitPlayerFullscreen();
    await settle();
    expect(plyr.fullscreen.exit).toHaveBeenCalledTimes(1);
    expect(h.invoke).not.toHaveBeenCalledWith('set_window_fullscreen', expect.anything());
  });

  it('with no player in full screen (mid hand-off) ends the window full screen directly', async () => {
    fullscreenMedia = null;
    exitPlayerFullscreen();
    await settle();
    expect(h.store.isPlayerFullscreen).toBe(false);
    expect(h.invoke).toHaveBeenCalledWith('set_window_fullscreen', { fullscreen: false });
  });

  it('a player whose Plyr is no longer active takes the direct path too', async () => {
    plyr.fullscreen.active = false;
    exitPlayerFullscreen();
    await settle();
    expect(plyr.fullscreen.exit).not.toHaveBeenCalled();
    expect(h.invoke).toHaveBeenCalledWith('set_window_fullscreen', { fullscreen: false });
  });

  it("leaves the window full screen alone when the app's own full screen owns it", async () => {
    fullscreenMedia = null;
    h.store.isWindowFullscreen = true;
    exitPlayerFullscreen();
    await settle();
    expect(h.store.isPlayerFullscreen).toBe(false);
    expect(h.invoke).not.toHaveBeenCalledWith('set_window_fullscreen', expect.anything());
  });
});

describe('handleTitleBarMouseDown', () => {
  const press = (detail: number, opts: { child?: boolean; button?: number } = {}) => {
    const currentTarget = {};
    return {
      button: opts.button ?? 0,
      detail,
      target: opts.child ? {} : currentTarget,
      currentTarget,
      screenX: 100,
      screenY: 10,
      preventDefault: vi.fn(),
    } as unknown as Parameters<typeof handleTitleBarMouseDown>[0];
  };

  it('double-click maximizes outside full screen', async () => {
    handleTitleBarMouseDown(press(2), false);
    await settle();
    expect(h.win.toggleMaximize).toHaveBeenCalledTimes(1);
  });

  it('double-click does nothing in full screen', async () => {
    handleTitleBarMouseDown(press(2), false, { fullscreen: true });
    await settle();
    expect(h.win.toggleMaximize).not.toHaveBeenCalled();
    expect(h.invoke).not.toHaveBeenCalled();
  });

  it('a press on a restored window hands it to the OS drag at once', () => {
    handleTitleBarMouseDown(press(1), false);
    expect(h.invoke).toHaveBeenCalledWith('start_titlebar_drag', undefined);
    expect(isTitlebarDragActive()).toBe(true);
  });

  it('a press on a maximized window waits for real movement, then cleans up', () => {
    handleTitleBarMouseDown(press(1), true);
    expect(h.invoke).not.toHaveBeenCalled();
    fire('mousemove', { screenX: 102, screenY: 11 });
    expect(h.invoke).not.toHaveBeenCalled();
    fire('mousemove', { screenX: 120, screenY: 10 });
    expect(h.invoke).toHaveBeenCalledWith('start_titlebar_drag', undefined);
    expect(windowListeners.get('mousemove')?.size ?? 0).toBe(0);
    expect(windowListeners.get('mouseup')?.size ?? 0).toBe(0);
    expect(windowListeners.get('blur')?.size ?? 0).toBe(0);
  });

  it('a click on a maximized window that never moves leaves no listeners behind', () => {
    handleTitleBarMouseDown(press(1), true);
    fire('mouseup');
    expect(h.invoke).not.toHaveBeenCalled();
    expect(windowListeners.get('mousemove')?.size ?? 0).toBe(0);
  });

  it('presses on a child control, other buttons and triple-clicks are ignored', () => {
    handleTitleBarMouseDown(press(1, { child: true }), false);
    handleTitleBarMouseDown(press(1, { button: 2 }), false);
    handleTitleBarMouseDown(press(3), false);
    expect(h.invoke).not.toHaveBeenCalled();
  });

  it('the drag stays active until the native end signal plus its grace', () => {
    vi.useFakeTimers();
    vi.setSystemTime(1_000_000);
    handleTitleBarMouseDown(press(1), false);
    expect(isTitlebarDragActive()).toBe(true);
    vi.setSystemTime(1_060_000);
    expect(isTitlebarDragActive()).toBe(true);
    for (const cb of h.listeners['titlebar-drag-ended'] ?? []) cb();
    vi.setSystemTime(1_060_200);
    expect(isTitlebarDragActive()).toBe(true);
    vi.setSystemTime(1_060_400);
    expect(isTitlebarDragActive()).toBe(false);
  });

  // A full-screen window fills its monitor, and Windows only snaps a moved
  // full-screen window back when the monitor changes, so a press that moved a
  // few pixels used to pull it off its edge.
  it('a press in full screen does not hand the window to the OS move loop', () => {
    handleTitleBarMouseDown(press(1), false, { fullscreen: true });
    handleTitleBarMouseDown(press(1), true, { fullscreen: true });
    fire('mousemove', { screenX: 160, screenY: 40 });
    expect(h.invoke).not.toHaveBeenCalled();
    expect(windowListeners.get('mousemove')?.size ?? 0).toBe(0);
  });
});
