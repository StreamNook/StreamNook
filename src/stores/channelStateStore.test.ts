import { describe, expect, it, vi } from 'vitest';
import type { ChannelState } from '../types';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));

import { invoke } from '@tauri-apps/api/core';
import { newestSections, useChannelStateStore, watchChannel } from './channelStateStore';

const blank = (login: string): ChannelState => ({
  login,
  channel_id: '1',
  viewer_count: null,
  started_at: null,
  title: null,
  game_name: null,
  viewers_at: null,
  points: null,
  points_at: null,
  pinned: [],
  pinned_at: null,
  collab: null,
  collab_at: null,
});

describe('channel state store', () => {
  it('keeps a live update that arrives before the watch call answers', async () => {
    let answer!: (s: ChannelState) => void;
    vi.mocked(invoke).mockImplementation(() => new Promise((r) => (answer = r as (s: ChannelState) => void)));
    const watching = watchChannel('summit1g', '26490481', true);
    // Rust's first fetch, announced while the watch call is still in flight.
    useChannelStateStore.getState().apply({
      section: 'viewers',
      login: 'summit1g',
      viewer_count: 5000,
      started_at: '2026-10-03T20:00:00Z',
      title: 'live',
      game_name: 'Just Chatting',
      at: 10,
    });
    answer(blank('summit1g'));
    await watching;
    const s = useChannelStateStore.getState().channels.get('summit1g');
    expect(s?.viewer_count).toBe(5000);
    expect(s?.viewers_at).toBe(10);
  });

  it('ignores updates for channels this window does not watch', () => {
    useChannelStateStore.getState().apply({ section: 'points', login: 'nobody', points: null, at: 1 });
    expect(useChannelStateStore.getState().channels.has('nobody')).toBe(false);
  });

  it('a newer answer replaces an older section', () => {
    const prev = { ...blank('a'), viewer_count: 1, viewers_at: 5 };
    const incoming = { ...blank('a'), viewer_count: 2, viewers_at: 9 };
    expect(newestSections(prev, incoming).viewer_count).toBe(2);
    expect(newestSections(incoming, prev).viewer_count).toBe(2);
  });
});
