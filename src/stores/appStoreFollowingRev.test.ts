import { beforeEach, describe, expect, it, vi } from 'vitest';

// Rust stamps every Following push and snapshot with a revision. A push
// applies only when newer than what the window holds; a snapshot (built
// under the same lock, so as new as every push at or below its revision)
// applies when at least as new. A late push or read never rolls the
// favorite labels back.

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}), emit: vi.fn() }));
vi.mock('../bootPreload', () => ({ takePreloadedSettings: () => null }));

import { useAppStore } from './AppStore';
import type { HomeSnapshot, Settings } from '../types';

const lists = (ids: string[]) => ({
  favorites: [],
  live: [],
  offline: [],
  followed_live: [],
  platforms: [],
  favorite_ids: ids,
  favorites_list: [],
});

function event(ids: string[], rev: number) {
  useAppStore.getState().applyHomeUpdate({ section: 'following', ...lists(ids), rev, at: 0 });
}

function snapshot(ids: string[], rev: number) {
  useAppStore.getState().applyHomeSnapshot({
    followed_live_at: null,
    offline_at: null,
    recommended_at: null,
    hype_at: null,
    streaks_at: null,
    drops_at: null,
    continue_watching_at: null,
    unified_discover_at: null,
    following: lists(ids),
    following_at: 1,
    following_rev: rev,
  } as unknown as HomeSnapshot);
}

const ids = () => useAppStore.getState().following.favorite_ids;

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
  useAppStore.setState({ followingRev: -1, followingApplied: false, favoriteVersion: 0, following: lists([]) });
});

describe('following revisions', () => {
  it('a push applies only when newer than the one held', () => {
    event(['1'], 5);
    expect(ids()).toEqual(['1']);
    expect(useAppStore.getState().followingApplied).toBe(true);
    event(['2'], 5);
    event(['3'], 4);
    expect(ids()).toEqual(['1']);
    event(['4'], 6);
    expect(ids()).toEqual(['4']);
  });

  it('a snapshot applies when at least as new, never older', () => {
    event(['1'], 5);
    snapshot(['2'], 5);
    expect(ids()).toEqual(['2']);
    snapshot(['3'], 4);
    expect(ids()).toEqual(['2']);
    // A push at the snapshot's own revision is older content: dropped.
    event(['4'], 5);
    expect(ids()).toEqual(['2']);
  });

  it('a snapshot alone marks the lists arrived', () => {
    snapshot([], 0);
    expect(useAppStore.getState().followingApplied).toBe(true);
  });

  it('a change of favorites moves the label version', () => {
    event(['1'], 1);
    const v = useAppStore.getState().favoriteVersion;
    event(['1'], 2);
    expect(useAppStore.getState().favoriteVersion).toBe(v);
    event(['1', '2'], 3);
    expect(useAppStore.getState().favoriteVersion).toBe(v + 1);
  });
});

describe('updateSettings', () => {
  it('never sends the Rust-owned lists and keeps the store copy of them', async () => {
    const base = useAppStore.getState().settings;
    useAppStore.setState({ settings: { ...base, favorite_streamers: ['new'], favorite_channels: [] } as Settings });
    const stale = { ...useAppStore.getState().settings, favorite_streamers: ['old'], chat_placement: 'test-placement' } as Settings;
    await useAppStore.getState().updateSettings(stale);
    const patch = invokeMock.mock.calls.find(([cmd]) => cmd === 'patch_settings')?.[1]?.patch as Record<string, unknown>;
    expect(patch).toBeDefined();
    expect(patch).not.toHaveProperty('favorite_streamers');
    expect(patch).toHaveProperty('chat_placement', 'test-placement');
    expect(useAppStore.getState().settings.favorite_streamers).toEqual(['new']);
    expect(useAppStore.getState().settings.chat_placement).toBe('test-placement');
  });

  it('a change only to the Rust-owned lists sends nothing', async () => {
    const current = useAppStore.getState().settings;
    await useAppStore.getState().updateSettings({ ...current, favorite_streamers: ['x'] } as Settings);
    expect(invokeMock.mock.calls.some(([cmd]) => cmd === 'patch_settings')).toBe(false);
  });
});
