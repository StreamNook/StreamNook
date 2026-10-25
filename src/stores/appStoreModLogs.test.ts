import { beforeEach, describe, expect, it, vi } from 'vitest';

// The mod-log history of a channel is loaded once per open. A load that fails
// (Rust could not read the saved history) must leave the channel loadable
// again, or the history stays missing until the app restarts.

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(),
}));

vi.mock('../bootPreload', () => ({ takePreloadedSettings: () => null }));

import { useAppStore } from './AppStore';

const entry = (id: string) => ({ id, channel: 'chan', action: 'ban', timestamp: '2026-10-24T00:00:00Z' });

beforeEach(() => {
  invokeMock.mockReset();
  useAppStore.setState({ modLogs: [], loadedModLogChannels: new Set<string>() });
});

describe('loadModLogsForChannel', () => {
  it('leaves the channel loadable again when the load is rejected', async () => {
    invokeMock.mockRejectedValueOnce('Failed to read mod logs file');
    await useAppStore.getState().loadModLogsForChannel('Chan');
    expect(useAppStore.getState().loadedModLogChannels.has('chan')).toBe(false);

    invokeMock.mockResolvedValueOnce([entry('a1')]);
    await useAppStore.getState().loadModLogsForChannel('Chan');
    expect(invokeMock).toHaveBeenCalledTimes(2);
    expect(useAppStore.getState().loadedModLogChannels.has('chan')).toBe(true);
    expect(useAppStore.getState().modLogs.map((l) => l.id)).toEqual(['a1']);
  });

  it('loads a channel that loaded fine only once', async () => {
    invokeMock.mockResolvedValue([entry('a1')]);
    await useAppStore.getState().loadModLogsForChannel('chan');
    await useAppStore.getState().loadModLogsForChannel('chan');
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });
});
