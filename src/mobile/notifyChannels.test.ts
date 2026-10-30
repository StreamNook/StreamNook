// The phone sends the state each press wants, in the order pressed. Rust owns
// the list (live_mute.rs, with its own tests); this pins the shell's half.
import { beforeEach, describe, expect, it, vi } from 'vitest';

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock('../stores/AppStore', () => ({
  useAppStore: { getState: () => ({ settings: { live_notifications: { muted_live_channels: ['alpha'] } } }) },
}));
vi.mock('../utils/logger', () => ({ Logger: { warn: vi.fn(), debug: vi.fn(), info: vi.fn() } }));

import * as notify from './notifyChannels';

const alpha = { provider: 'twitch', userId: '1', userLogin: 'Alpha' };

beforeEach(() => {
  invokeMock.mockReset();
});

describe('live alert presses', () => {
  it('apply in the order they were made, each with the state it wants', async () => {
    expect(notify.setLiveAlert).toBeTypeOf('function');
    let release: (v: boolean) => void = () => {};
    invokeMock.mockImplementationOnce(
      () =>
        new Promise<boolean>((r) => {
          release = r;
        }),
    );
    invokeMock.mockResolvedValueOnce(true);
    const first = notify.setLiveAlert(alpha, false);
    const second = notify.setLiveAlert(alpha, true);
    await Promise.resolve();
    await Promise.resolve();
    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenNthCalledWith(1, 'set_live_alert', {
      provider: 'twitch',
      userId: '1',
      userLogin: 'Alpha',
      on: false,
    });
    release(false);
    await expect(first).resolves.toBe(false);
    await expect(second).resolves.toBe(true);
    expect(invokeMock).toHaveBeenNthCalledWith(2, 'set_live_alert', {
      provider: 'twitch',
      userId: '1',
      userLogin: 'Alpha',
      on: true,
    });
  });

  it('keeps going after a failed press', async () => {
    invokeMock.mockRejectedValueOnce('boom');
    invokeMock.mockResolvedValueOnce(false);
    await expect(notify.setLiveAlert(alpha, false)).resolves.toBeNull();
    await expect(notify.setLiveAlert(alpha, false)).resolves.toBe(false);
  });

  it('reads the coded Copy errors and folds anything else to twitch-error', async () => {
    invokeMock.mockRejectedValueOnce('wrong-account');
    await expect(notify.copyTwitchBells()).resolves.toEqual({ ok: false, code: 'wrong-account' });
    invokeMock.mockRejectedValueOnce(new Error('socket closed'));
    await expect(notify.copyTwitchBells()).resolves.toEqual({ ok: false, code: 'twitch-error' });
    invokeMock.mockResolvedValueOnce(3);
    await expect(notify.undoTwitchBells(7)).resolves.toEqual({ ok: true, removed: 3 });
    expect(invokeMock).toHaveBeenLastCalledWith('undo_live_mutes', { copyId: 7 });
  });

  it('shows muted state from the store for display', () => {
    expect(notify.isChannelMuted('ALPHA')).toBe(true);
    expect(notify.isChannelMuted('bravo')).toBe(false);
  });

  it('matches a stored entry whatever its case, like Rust does', () => {
    expect(notify.mutedIncludes(['Kamet0'], 'kamet0')).toBe(true);
    expect(notify.mutedIncludes([' KAMET0 '], 'Kamet0')).toBe(true);
    expect(notify.mutedIncludes(['kamet0'], 'kamet')).toBe(false);
    expect(notify.mutedIncludes(['kick:kamet0'], 'kamet0')).toBe(false);
    expect(notify.mutedIncludes(undefined, 'kamet0')).toBe(false);
    expect(notify.mutedIncludes(['kamet0'], '')).toBe(false);
  });
});
