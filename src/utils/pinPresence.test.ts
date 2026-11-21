// @vitest-environment happy-dom
// SN-284: the window's presence rules. Turning Pin mentions off and on again
// while the first `listen` is still pending must leave one listener, not two.
import { beforeEach, describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({
  on: false,
  enabled: false,
  subs: [] as Array<() => void>,
  pending: [] as Array<{ resolve: () => void; off: ReturnType<typeof vi.fn> }>,
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(
    () =>
      new Promise<() => void>((resolve) => {
        const off = vi.fn();
        h.pending.push({ resolve: () => resolve(off), off });
      }),
  ),
}));
vi.mock('../stores/AppStore', () => ({
  useAppStore: {
    getState: () => ({ settings: { chat_design: { pin_mentions: h.on } } }),
    subscribe: (fn: () => void) => {
      h.subs.push(fn);
      return () => {};
    },
  },
}));
vi.mock('./pinInput', () => ({ setInputSideEffect: vi.fn(), setPinInputEnabled: vi.fn() }));
vi.mock('./pinSurfaces', () => ({
  pinSurfaces: {
    setEnabled: (on: boolean) => {
      h.enabled = on;
    },
    isEnabled: () => h.enabled,
    resend: vi.fn(),
  },
}));
vi.mock('./platform', () => ({ IS_MOBILE: false }));

const flip = (on: boolean) => {
  h.on = on;
  h.subs.forEach((fn) => fn());
};
const settle = async () => {
  await Promise.resolve();
  await Promise.resolve();
};

beforeEach(() => {
  h.on = false;
  h.enabled = false;
  h.subs = [];
  h.pending = [];
  vi.resetModules();
});

describe('startPinPresence', () => {
  it('lets go of a listener that was pending when Pin mentions flipped again', async () => {
    const { startPinPresence } = await import('./pinPresence');
    startPinPresence();
    flip(true);
    flip(false);
    flip(true);
    expect(h.pending).toHaveLength(2);
    // The first `listen` answers late, after the setting is on again.
    h.pending[0].resolve();
    h.pending[1].resolve();
    await settle();
    expect(h.pending[0].off, 'the stale listener is released').toHaveBeenCalledTimes(1);
    expect(h.pending[1].off, 'the current one is kept').not.toHaveBeenCalled();
    // Turning it off releases the one that is kept.
    flip(false);
    expect(h.pending[1].off).toHaveBeenCalledTimes(1);
  });

  it('releases a listener that answers after Pin mentions went off', async () => {
    const { startPinPresence } = await import('./pinPresence');
    startPinPresence();
    flip(true);
    flip(false);
    h.pending[0].resolve();
    await settle();
    expect(h.pending[0].off).toHaveBeenCalledTimes(1);
  });
});
