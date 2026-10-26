import { beforeEach, describe, expect, it, vi } from 'vitest';

// What the view does with the answer to "did the stream end": a confirmed end
// drops the stream's pause and the own-channel memory, any other answer leaves
// them alone.

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(),
}));

vi.mock('../bootPreload', () => ({ takePreloadedSettings: () => null }));

import { useAppStore } from './AppStore';
import { clearPauseIntent, forgetOwnChannel, heldPause, holdOwnChannel, notePauseIntent } from '../utils/pausePark';

const stream = { user_login: 'chan', user_name: 'Chan' };

/** A paused own-channel stream, as the player leaves it. */
function holdStream() {
  forgetOwnChannel();
  clearPauseIntent('main');
  expect(holdOwnChannel('twitch:chan')).toBe(true);
  notePauseIntent('main', 'twitch:chan');
  expect(heldPause('main', 'twitch:chan')).not.toBeNull();
}

async function answer(decision: Record<string, unknown>) {
  invokeMock.mockResolvedValueOnce(decision);
  await useAppStore.getState().handleStreamOffline();
}

beforeEach(() => {
  invokeMock.mockReset();
  useAppStore.setState({ currentStream: stream as never, addToast: vi.fn() as never });
  holdStream();
});

describe('applyOfflineDecision', () => {
  it('a confirmed end with auto-switch off clears the pause and the own-channel memory', async () => {
    await answer({ action: 'nothing', why: 'auto-switch is off', ended: true });
    expect(heldPause('main', 'twitch:chan')).toBeNull();
    expect(holdOwnChannel('twitch:chan')).toBe(true); // the memory is gone
  });

  it('a nothing answer that is not an end keeps the pause and the memory', async () => {
    await answer({ action: 'nothing', why: 'a newer stream started' });
    expect(heldPause('main', 'twitch:chan')).not.toBeNull();
    expect(holdOwnChannel('twitch:chan')).toBe(false);
    await answer({ action: 'nothing', why: 'already resolving', ended: false });
    expect(heldPause('main', 'twitch:chan')).not.toBeNull();
    expect(holdOwnChannel('twitch:chan')).toBe(false);
  });

  it('a notice with nothing stopped still clears the pause and the memory', async () => {
    await answer({ action: 'notify', notice: { message: 'chan went offline.', error: false } });
    expect(heldPause('main', 'twitch:chan')).toBeNull();
    expect(holdOwnChannel('twitch:chan')).toBe(true);
  });
});
