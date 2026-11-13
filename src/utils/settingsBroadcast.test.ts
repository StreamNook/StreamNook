import { beforeEach, describe, expect, it, vi } from 'vitest';

type Handler = (event: { payload: unknown }) => void;
const handlers = new Map<string, Handler>();
const unlistened: string[] = [];

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, handler: Handler) => {
    handlers.set(name, handler);
    return () => {
      unlistened.push(name);
    };
  }),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

import {
  DROPS_SETTINGS_UPDATED_EVENT,
  SENDER_ID,
  SETTINGS_UPDATED_EVENT,
  listenForSettingsUpdates,
} from './settingsBroadcast';

describe('listenForSettingsUpdates', () => {
  beforeEach(() => {
    handlers.clear();
    unlistened.length = 0;
  });

  it('refreshes on a drops settings save, so the Stop toast reads the live Missing badges switch', async () => {
    const onUpdate = vi.fn();
    await listenForSettingsUpdates(onUpdate);
    handlers.get(DROPS_SETTINGS_UPDATED_EVENT)?.({ payload: { collect_missing_badges: true } });
    expect(onUpdate).toHaveBeenCalledTimes(1);
  });

  it("refreshes on another window's save and ignores its own", async () => {
    const onUpdate = vi.fn();
    await listenForSettingsUpdates(onUpdate);
    handlers.get(SETTINGS_UPDATED_EVENT)?.({ payload: { source: SENDER_ID, keys: [] } });
    expect(onUpdate).not.toHaveBeenCalled();
    handlers.get(SETTINGS_UPDATED_EVENT)?.({ payload: { source: 'another-window', keys: ['drops'] } });
    expect(onUpdate).toHaveBeenCalledTimes(1);
  });

  it('detaches from both events', async () => {
    const unlisten = await listenForSettingsUpdates(() => {});
    unlisten();
    expect(unlistened.sort()).toEqual([DROPS_SETTINGS_UPDATED_EVENT, SETTINGS_UPDATED_EVENT].sort());
  });
});
