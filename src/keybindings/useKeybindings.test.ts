// @vitest-environment node
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { BindableCommand } from './types';

// The dispatcher's deferred matches: a key the page answers too (Esc) runs its
// command only at the end of the press, and only if nothing on the way marked
// the key handled. Ordinary matches keep being swallowed in the capture pass.

const h = vi.hoisted(() => ({
  commands: [] as BindableCommand[],
  store: {} as Record<string, unknown>,
}));

vi.mock('../stores/AppStore', () => ({ useAppStore: { getState: () => h.store } }));
vi.mock('./commands', () => ({ getBindableCommands: () => h.commands }));
vi.mock('./registry', () => ({
  getOverrides: () => ({}),
  getEffectiveBindings: (cmd: BindableCommand) => cmd.defaultBindings,
}));
vi.mock('./playerControls', () => ({ isPlayerControllable: () => false }));
vi.mock('./chatModController', () => ({ getChatModController: () => null }));
vi.mock('./recorderState', () => ({ isRecordingKeybind: () => false }));

// isEditableTarget asks `instanceof HTMLElement`; nothing here is one.
(globalThis as Record<string, unknown>).HTMLElement ??= class {};

const { createKeybindingDispatcher } = await import('./useKeybindings');

/** The window's bubble listeners, in the order a browser would call them. */
class FakeWindow {
  listeners: { fn: (e: Event) => void; once: boolean }[] = [];
  addEventListener(_type: string, fn: EventListenerOrEventListenerObject, opts?: boolean | AddEventListenerOptions) {
    const once = typeof opts === 'object' && !!opts.once;
    this.listeners.push({ fn: fn as (e: Event) => void, once });
  }
  bubble(e: Event) {
    for (const l of [...this.listeners]) {
      if (l.once) this.listeners = this.listeners.filter((x) => x !== l);
      l.fn(e);
    }
  }
}

const press = (code: string, key: string) => {
  const e = {
    code,
    key,
    keyCode: 0,
    repeat: false,
    isComposing: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    metaKey: false,
    target: null,
    defaultPrevented: false,
    propagationStopped: false,
    getModifierState: () => false,
    preventDefault() {
      this.defaultPrevented = true;
    },
    stopPropagation() {
      this.propagationStopped = true;
    },
  };
  return e;
};
type Press = ReturnType<typeof press>;

/** One key press through the page: the capture pass, whatever the page does
 *  with the key, then the window's bubble pass unless propagation stopped. */
const dispatch = (win: FakeWindow, capture: (e: KeyboardEvent) => void, e: Press, page?: (e: Press) => void) => {
  capture(e as unknown as KeyboardEvent);
  if (e.propagationStopped) return;
  page?.(e);
  if (e.propagationStopped) return;
  win.bubble(e as unknown as Event);
};

const exitFullscreen = vi.fn();
const toggleMute = vi.fn();
let available = true;

beforeEach(() => {
  exitFullscreen.mockClear();
  toggleMute.mockClear();
  available = true;
  h.store = {};
  h.commands = [
    {
      id: 'player.exitFullscreen',
      label: 'Exit full screen',
      category: 'Player',
      context: 'global',
      defaultBindings: ['Esc'],
      deferred: true,
      isAvailable: () => available,
      run: exitFullscreen,
    },
    {
      id: 'player.mute',
      label: 'Mute',
      category: 'Player',
      context: 'global',
      defaultBindings: ['M'],
      run: toggleMute,
    },
  ];
});

describe('deferred bindings', () => {
  it('run once the page has had the key, leaving it unswallowed on the way', () => {
    const win = new FakeWindow();
    const capture = createKeybindingDispatcher(win as unknown as Window);
    const e = press('Escape', 'Escape');
    let seenByPage = false;
    dispatch(win, capture, e, () => {
      seenByPage = true;
      expect(exitFullscreen).not.toHaveBeenCalled();
    });
    expect(seenByPage).toBe(true);
    expect(exitFullscreen).toHaveBeenCalledTimes(1);
    expect(e.propagationStopped).toBe(false);
  });

  it('stand aside for a handler that marked the key handled', () => {
    const win = new FakeWindow();
    const capture = createKeybindingDispatcher(win as unknown as Window);
    dispatch(win, capture, press('Escape', 'Escape'), (e) => e.preventDefault());
    expect(exitFullscreen).not.toHaveBeenCalled();
  });

  it("run after the window's own listeners, so one of those can claim the key too", () => {
    const win = new FakeWindow();
    const capture = createKeybindingDispatcher(win as unknown as Window);
    // A window listener registered long before this press (a tile menu).
    win.addEventListener('keydown', (e) => e.preventDefault());
    dispatch(win, capture, press('Escape', 'Escape'));
    expect(exitFullscreen).not.toHaveBeenCalled();
  });

  it('never run for a press that stopped on the way, and the next press is unaffected', () => {
    const win = new FakeWindow();
    const capture = createKeybindingDispatcher(win as unknown as Window);
    dispatch(win, capture, press('Escape', 'Escape'), (e) => e.stopPropagation());
    expect(exitFullscreen).not.toHaveBeenCalled();
    dispatch(win, capture, press('Escape', 'Escape'));
    expect(exitFullscreen).toHaveBeenCalledTimes(1);
  });

  it('never match while something else owns Esc', () => {
    available = false;
    const win = new FakeWindow();
    const capture = createKeybindingDispatcher(win as unknown as Window);
    const e = press('Escape', 'Escape');
    dispatch(win, capture, e);
    expect(exitFullscreen).not.toHaveBeenCalled();
    expect(win.listeners).toHaveLength(0);
  });

  it('stand down with a surface that owns the keyboard open', () => {
    h.store = { isSettingsOpen: true };
    const win = new FakeWindow();
    const capture = createKeybindingDispatcher(win as unknown as Window);
    dispatch(win, capture, press('Escape', 'Escape'));
    expect(exitFullscreen).not.toHaveBeenCalled();
  });
});

describe('ordinary bindings', () => {
  it('are still swallowed and run in the capture pass', () => {
    const win = new FakeWindow();
    const capture = createKeybindingDispatcher(win as unknown as Window);
    const e = press('KeyM', 'm');
    const page = vi.fn();
    dispatch(win, capture, e, page);
    expect(toggleMute).toHaveBeenCalledTimes(1);
    expect(e.propagationStopped).toBe(true);
    expect(e.defaultPrevented).toBe(true);
    expect(page).not.toHaveBeenCalled();
  });
});
