import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type Plyr from 'plyr';

// A stream switch destroys the full-screen player and builds another. These
// check that full screen passes to the new player, and ends when none comes.

const win = {
  isFullscreen: vi.fn(async () => true),
  isMaximized: vi.fn(async () => false),
  unmaximize: vi.fn(async () => {}),
  setFullscreen: vi.fn(async (_on: boolean) => {}),
  setPosition: vi.fn(async () => {}),
  maximize: vi.fn(async () => {}),
};
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => win,
  currentMonitor: async () => null,
  PhysicalPosition: class {
    constructor(
      public x: number,
      public y: number,
    ) {}
  },
}));
const state = { isPlayerFullscreen: true, isWindowFullscreen: false, playerOverlayVisible: true };
vi.mock('../stores/AppStore', () => ({
  useAppStore: {
    getState: () => state,
    setState: (p: Partial<typeof state>) => Object.assign(state, p),
  },
}));
vi.mock('./platform', () => ({ IS_MAC: false }));
vi.mock('./logger', () => ({ Logger: { error: vi.fn(), debug: vi.fn(), warn: vi.fn() } }));

import { claimPlayerFullscreen, handOffPlayerFullscreen } from './windowFullscreen';

type FakePlayer = Plyr & { fullscreen: { active: boolean; enter: ReturnType<typeof vi.fn> } };
const fakePlayer = (active: boolean) => ({ fullscreen: { active, enter: vi.fn() } }) as unknown as FakePlayer;

describe('player full screen across a stream switch', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    state.isPlayerFullscreen = true;
    win.setFullscreen.mockClear();
  });
  afterEach(() => vi.useRealTimers());

  it('the next player takes full screen over', async () => {
    handOffPlayerFullscreen(fakePlayer(true), { nextPlayerComing: true });
    const next = fakePlayer(false);
    claimPlayerFullscreen(next);
    expect(next.fullscreen.enter).toHaveBeenCalledOnce();
    await vi.advanceTimersByTimeAsync(10_000);
    await vi.dynamicImportSettled();
    expect(win.setFullscreen).not.toHaveBeenCalledWith(false);
    expect(state.isPlayerFullscreen).toBe(true);
  });

  it('ends full screen when no player follows in time', async () => {
    handOffPlayerFullscreen(fakePlayer(true), { nextPlayerComing: true });
    await vi.advanceTimersByTimeAsync(4_000);
    await vi.dynamicImportSettled();
    expect(state.isPlayerFullscreen).toBe(false);
    expect(win.setFullscreen).toHaveBeenCalledWith(false);
  });

  it('ends full screen at once when the stream stopped', async () => {
    handOffPlayerFullscreen(fakePlayer(true), { nextPlayerComing: false });
    await vi.advanceTimersByTimeAsync(0);
    await vi.dynamicImportSettled();
    expect(state.isPlayerFullscreen).toBe(false);
  });

  it('a player that was not full screen hands nothing over', () => {
    handOffPlayerFullscreen(fakePlayer(false), { nextPlayerComing: true });
    const next = fakePlayer(false);
    claimPlayerFullscreen(next);
    expect(next.fullscreen.enter).not.toHaveBeenCalled();
  });
});
