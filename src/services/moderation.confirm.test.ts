import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// AppStore touches `window` at module load; the bridge only needs its toasts.
const addToast = vi.fn();
const removeToast = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('../stores/AppStore', () => ({
  useAppStore: Object.assign(vi.fn(), {
    getState: () => ({ addToast, removeToast }),
    setState: vi.fn(),
    subscribe: vi.fn(),
  }),
}));

import { invoke } from '@tauri-apps/api/core';
import { armedFor, moderateWithToast, type ModOutcome } from './moderation';

const answer = (o: ModOutcome) => vi.mocked(invoke).mockResolvedValueOnce(o);
const ban = { broadcasterId: '1', targetUserId: '9', targetName: 'gus', action: { kind: 'ban' as const }, onePress: true };

describe('Ban confirmation', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    addToast.mockClear();
    removeToast.mockClear();
  });
  afterEach(() => vi.useRealTimers());

  it('shows the first press prompt even with toasts off, then takes it down when the window closes', async () => {
    answer({ appliedSeconds: null, message: 'Press again within 3 seconds to ban gus', undoable: false, confirmMs: 3000 });
    const out = await moderateWithToast(ban);
    expect(out?.confirmMs).toBe(3000);
    expect(addToast).toHaveBeenCalledTimes(1);
    const [message, type, action, options] = addToast.mock.calls[0];
    expect(message).toBe('Press again within 3 seconds to ban gus');
    expect(type).toBe('info');
    expect(action).toBeUndefined();
    expect(options).toMatchObject({ alwaysShow: true, skipIsland: true });
    vi.advanceTimersByTime(3000);
    expect(removeToast).toHaveBeenCalledWith(options.id);
  });

  it('says nothing again for a press too soon after the first', async () => {
    answer({ appliedSeconds: null, message: '', undoable: false, confirmMs: 2800 });
    await moderateWithToast(ban);
    expect(addToast).not.toHaveBeenCalled();
  });

  it('a real ban still gets its words and Undo', async () => {
    answer({ appliedSeconds: null, message: 'Banned gus', undoable: true, confirmMs: null });
    await moderateWithToast(ban);
    expect(addToast).toHaveBeenCalledTimes(1);
    expect(addToast.mock.calls[0][2]).toMatchObject({ label: 'Undo' });
  });

  it('a surface shows the arm a little shorter than Rust waits', () => {
    expect(armedFor({ appliedSeconds: null, message: '', undoable: false, confirmMs: 3000 })).toBe(2900);
    expect(armedFor({ appliedSeconds: null, message: '', undoable: false, confirmMs: null })).toBe(0);
  });
});
