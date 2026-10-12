import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type Plyr from 'plyr';

// A stream switch destroys the full-screen player and builds another. These
// check that full screen passes to the new player, and ends when none comes.

// Rust owns the window's side of full screen (set_window_fullscreen), so the
// page's only seam to the window is this one command.
const { invoke } = vi.hoisted(() => ({
  invoke: vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => true),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: unknown) => invoke(cmd, args) }));
const leaveWindowFullscreen = ['set_window_fullscreen', { fullscreen: false }] as const;
const state = { isPlayerFullscreen: true, isWindowFullscreen: false, playerOverlayVisible: true };
vi.mock('../stores/AppStore', () => ({
  useAppStore: {
    getState: () => state,
    setState: (p: Partial<typeof state>) => Object.assign(state, p),
  },
}));
vi.mock('./logger', () => ({ Logger: { error: vi.fn(), debug: vi.fn(), warn: vi.fn() } }));

import { claimPlayerFullscreen, handOffPlayerFullscreen, setWindowFullscreen, syncTauriWindowFullscreen } from './windowFullscreen';

type FakePlayer = Plyr & { fullscreen: { active: boolean; enter: ReturnType<typeof vi.fn> } };
const fakePlayer = (active: boolean) => ({ fullscreen: { active, enter: vi.fn() } }) as unknown as FakePlayer;

describe('player full screen across a stream switch', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    state.isPlayerFullscreen = true;
    state.isWindowFullscreen = false;
    invoke.mockClear();
  });
  afterEach(() => vi.useRealTimers());

  it('the next player takes full screen over', async () => {
    handOffPlayerFullscreen(fakePlayer(true), { nextPlayerComing: true });
    const next = fakePlayer(false);
    claimPlayerFullscreen(next);
    expect(next.fullscreen.enter).toHaveBeenCalledOnce();
    await vi.advanceTimersByTimeAsync(10_000);
    await vi.dynamicImportSettled();
    expect(invoke).not.toHaveBeenCalledWith(...leaveWindowFullscreen);
    expect(state.isPlayerFullscreen).toBe(true);
  });

  it('ends full screen when no player follows in time', async () => {
    handOffPlayerFullscreen(fakePlayer(true), { nextPlayerComing: true });
    await vi.advanceTimersByTimeAsync(4_000);
    await vi.dynamicImportSettled();
    expect(state.isPlayerFullscreen).toBe(false);
    expect(invoke).toHaveBeenCalledWith(...leaveWindowFullscreen);
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

// Both full screens go through one Rust command, which keeps the window's
// maximize across the trip; the page only says which way.
describe('the window side of full screen', () => {
  beforeEach(() => {
    state.isWindowFullscreen = false;
    invoke.mockClear();
  });

  it("the player's full screen asks Rust to enter and to leave", async () => {
    await syncTauriWindowFullscreen(true);
    expect(invoke).toHaveBeenLastCalledWith('set_window_fullscreen', { fullscreen: true });
    await syncTauriWindowFullscreen(false);
    expect(invoke).toHaveBeenLastCalledWith(...leaveWindowFullscreen);
  });

  it("leaving the player's full screen keeps the app's own full screen", async () => {
    state.isWindowFullscreen = true;
    await syncTauriWindowFullscreen(false);
    expect(invoke).not.toHaveBeenCalled();
    expect(state.isPlayerFullscreen).toBe(false);
  });

  it('with no direction it toggles, and answers what Rust reports', async () => {
    invoke.mockResolvedValueOnce(false);
    await expect(setWindowFullscreen()).resolves.toBe(false);
    expect(invoke).toHaveBeenCalledWith('set_window_fullscreen', { fullscreen: null });
  });
});
