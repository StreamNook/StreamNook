// @vitest-environment happy-dom
// SN-284: the phone tells Rust whether the app can be read (`set_app_interactive`),
// which the pinned mentions' clocks follow. What Rust holds must be what the phone
// last saw: a failed report is sent again on the next event, one report is in
// flight at a time, and a value that changed in flight is sent as soon as it can be.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({
  calls: [] as Array<{ cmd: string; interactive?: boolean; ok: () => void; fail: () => void }>,
  auto: null as null | 'ok' | 'fail',
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn((cmd: string, args?: { interactive?: boolean }) => {
    if (cmd !== 'set_app_interactive') return Promise.resolve(null);
    return new Promise((resolve, reject) => {
      const call = { cmd, interactive: args?.interactive, ok: () => resolve(null), fail: () => reject(new Error('refused')) };
      h.calls.push(call);
      if (h.auto === 'ok') call.ok();
      if (h.auto === 'fail') call.fail();
    });
  }),
}));
vi.mock('../services/supabaseService', () => ({
  refreshEntitlementRegistries: vi.fn(),
  resumePresence: vi.fn(async () => {}),
  suspendPresence: vi.fn(),
}));
vi.mock('./followRefresh', () => ({ refreshFollowingIfStale: vi.fn(async () => {}) }));
vi.mock('./nativeBridge', () => ({ isInPip: vi.fn(() => null), runNotifyCheckNow: vi.fn() }));
vi.mock('./backgroundGate', () => ({ setBackgrounded: vi.fn() }));
vi.mock('./chat/chatResume', () => ({ resumeChats: vi.fn(async () => {}) }));
vi.mock('../utils/logger', () => ({ Logger: { debug: vi.fn(), warn: vi.fn() } }));

type Life = typeof import('./lifecycle');
let life: Life;
let teardown: () => void;

const settle = async () => {
  for (let i = 0; i < 6; i++) await Promise.resolve();
};
const sent = () => h.calls.map((c) => c.interactive);
const setVisibility = (v: 'visible' | 'hidden') => {
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => v });
};

beforeEach(async () => {
  h.calls = [];
  h.auto = null;
  setVisibility('visible');
  delete document.documentElement.dataset.snPip;
  vi.resetModules();
  life = await import('./lifecycle');
});
afterEach(() => {
  teardown?.();
});

describe('set_app_interactive', () => {
  it('sends the first report when the lifecycle starts, one at a time', async () => {
    teardown = life.installLifecycle();
    expect(sent()).toEqual([true]);
    // Nothing else goes while it is in flight, whatever the events say.
    window.dispatchEvent(new Event('sn:pip'));
    expect(sent()).toEqual([true]);
    h.calls[0].ok();
    await settle();
    expect(sent()).toEqual([true]);
  });

  it.each([
    ['sn:pip', () => window.dispatchEvent(new Event('sn:pip'))],
    ['sn:screen-on', () => window.dispatchEvent(new Event('sn:screen-on'))],
    [
      'visibilitychange',
      () => {
        setVisibility('visible');
        document.dispatchEvent(new Event('visibilitychange'));
      },
    ],
    ['viewer input', () => life.retryInteractiveReport()],
  ])('sends a failed report again on %s', async (_name, fire) => {
    teardown = life.installLifecycle();
    h.calls[0].fail();
    await settle();
    expect(sent()).toEqual([true]);
    fire();
    await settle();
    expect(sent()).toEqual([true, true]);
    h.calls[1].ok();
    await settle();
    // Told: nothing more to send for the same value.
    fire();
    await settle();
    expect(sent()).toEqual([true, true]);
  });

  it('sends a value that changed in flight once the first one lands', async () => {
    teardown = life.installLifecycle();
    // The screen goes off while the first report is still on its way.
    window.dispatchEvent(new Event('sn:screen-off'));
    expect(sent()).toEqual([true]);
    h.calls[0].ok();
    await settle();
    expect(sent()).toEqual([true, false]);
  });

  it('sends a value that changed in flight even when the first one failed', async () => {
    teardown = life.installLifecycle();
    window.dispatchEvent(new Event('sn:screen-off'));
    h.calls[0].fail();
    await settle();
    expect(sent()).toEqual([true, false]);
  });

  it('does not ask a refusing core again for the same value on its own', async () => {
    h.auto = 'fail';
    teardown = life.installLifecycle();
    await settle();
    expect(sent()).toEqual([true]);
  });

  it('a report that landed before the change and one that lands after end on the latest value', async () => {
    teardown = life.installLifecycle();
    h.calls[0].ok();
    await settle();
    window.dispatchEvent(new Event('sn:screen-off'));
    expect(sent()).toEqual([true, false]);
    // The screen is back on before the `false` lands.
    setVisibility('visible');
    window.dispatchEvent(new Event('sn:screen-on'));
    h.calls[1].ok();
    await settle();
    expect(sent()).toEqual([true, false, true]);
  });
});
