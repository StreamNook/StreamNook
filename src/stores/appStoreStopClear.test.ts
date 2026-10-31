import { beforeEach, describe, expect, it, vi } from 'vitest';

// A close always ends with the stream cleared, and a raid follows only a stream
// still open in the layer.

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(),
}));

vi.mock('../bootPreload', () => ({ takePreloadedSettings: () => null }));

import { raidFollowAllowed, useAppStore } from './AppStore';

const stream = { user_login: 'chan', user_name: 'Chan', user_id: '1' };

beforeEach(() => {
  invokeMock.mockReset();
  useAppStore.setState({ currentStream: stream as never, streamUrl: 'http://127.0.0.1:9/live.m3u8' });
});

describe('stopStream', () => {
  it('clears the stream when stopping chat fails', async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === 'stop_chat') throw new Error('chat start lock');
      return undefined;
    });
    await useAppStore.getState().stopStream();
    expect(useAppStore.getState().currentStream).toBeNull();
    expect(useAppStore.getState().streamUrl).toBeNull();
  });

  it('clears the stream when every step answers', async () => {
    invokeMock.mockResolvedValue(true);
    await useAppStore.getState().stopStream();
    expect(useAppStore.getState().currentStream).toBeNull();
  });
});

describe('raidFollowAllowed', () => {
  it('follows a raid only while the layer is up', () => {
    const open = { currentStream: stream as never, streamUrl: 'http://x/live.m3u8', isLoading: false };
    expect(raidFollowAllowed(open)).toBe(true);
    expect(raidFollowAllowed({ ...open, streamUrl: null, isLoading: true })).toBe(true);
    expect(raidFollowAllowed({ ...open, streamUrl: null })).toBe(false);
    expect(raidFollowAllowed({ ...open, currentStream: null })).toBe(false);
  });
});
