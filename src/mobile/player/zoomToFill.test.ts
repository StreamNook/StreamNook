// The one writer of Zoom to fill (SN-30): shown at once and held while its
// writes are out, saved as one ordered field, the store leading again once they
// settle, and a failure put back and said once.
import { beforeEach, describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => {
  type Group = { zoom_to_fill?: boolean; autoplay?: boolean };
  type State = { settings: { video_player?: Group }; addToast: (message: string, kind: string) => void };
  const listeners = new Set<(s: State) => void>();
  const toasts: string[] = [];
  let state: State = { settings: {}, addToast: (message) => void toasts.push(message) };
  const useAppStore = Object.assign((select: (s: State) => unknown) => select(state), {
    getState: () => state,
    setState: (next: Partial<State> | ((s: State) => Partial<State>)) => {
      state = { ...state, ...(typeof next === 'function' ? next(state) : next) };
      listeners.forEach((l) => l(state));
    },
    subscribe: (l: (s: State) => void) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
  });
  const writes: {
    changes: Record<string, unknown>;
    order?: { stream: string; n: number };
    resolve: () => void;
    reject: (e: unknown) => void;
  }[] = [];
  return {
    useAppStore,
    toasts,
    writes,
    reset(settings: State['settings']) {
      state = { ...state, settings };
      listeners.clear();
      toasts.length = 0;
      writes.length = 0;
    },
  };
});

vi.mock('../../stores/AppStore', () => ({ useAppStore: h.useAppStore }));
vi.mock('../../utils/logger', () => ({ Logger: { warn: vi.fn() } }));
vi.mock('../../utils/settingsFields', () => ({
  setSettingsFields: (changes: Record<string, unknown>, order?: { stream: string; n: number }) =>
    new Promise<void>((resolve, reject) => {
      h.writes.push({ changes, order, resolve, reject });
    }),
}));

let mod: typeof import('./zoomToFill');

beforeEach(async () => {
  vi.resetModules();
  h.reset({ video_player: { zoom_to_fill: false, autoplay: true } });
  mod = await import('./zoomToFill');
});

describe('zoom to fill writer', () => {
  it('shows the choice at once, patches the store and writes one ordered field', async () => {
    const done = mod.writeZoomToFill(true);
    expect(mod.currentZoomChoice()).toBe(true);
    expect(h.useAppStore.getState().settings.video_player).toEqual({ zoom_to_fill: true, autoplay: true });
    expect(h.writes[0].changes).toEqual({ 'video_player.zoom_to_fill': true });
    expect(h.writes[0].order?.stream).toMatch(/^zoom-to-fill:/);
    expect(h.writes[0].order?.n).toBe(1);
    h.writes[0].resolve();
    await done;
    expect(mod.currentZoomChoice()).toBeNull();
    expect(h.useAppStore.getState().settings.video_player?.zoom_to_fill).toBe(true);
  });

  it('holds the choice through a store change mid-write, then lets the store lead', async () => {
    const done = mod.writeZoomToFill(true);
    h.useAppStore.setState({ settings: { video_player: { zoom_to_fill: false } } });
    expect(mod.currentZoomChoice()).toBe(true);
    h.writes[0].resolve();
    await done;
    expect(mod.currentZoomChoice()).toBeNull();
  });

  it('a failed newest write puts the store back and says so once', async () => {
    const done = mod.writeZoomToFill(true);
    h.writes[0].reject(new Error('disk'));
    await done;
    expect(mod.currentZoomChoice()).toBeNull();
    expect(h.useAppStore.getState().settings.video_player?.zoom_to_fill).toBe(false);
    expect(h.toasts).toEqual(['Could not save that change.']);
  });

  it('an older write failing never touches the newer choice', async () => {
    const first = mod.writeZoomToFill(true);
    const second = mod.writeZoomToFill(false);
    h.writes[0].reject(new Error('late'));
    await first;
    expect(mod.currentZoomChoice()).toBe(false);
    expect(h.toasts).toEqual([]);
    expect(h.writes[1].order?.n).toBe(2);
    h.writes[1].resolve();
    await second;
    expect(mod.currentZoomChoice()).toBeNull();
  });

  it('leaves the store alone when it has no video_player group', async () => {
    h.reset({});
    const done = mod.writeZoomToFill(true);
    expect(h.useAppStore.getState().settings.video_player).toBeUndefined();
    h.writes[0].resolve();
    await done;
  });
});
