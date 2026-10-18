// @vitest-environment node
// Run with: npm test
//
// The chat menus' follow and subscribe state. While a chat is open on a Twitch
// channel, Rust's channel state holds the viewer's relationship, so the very
// first render already shows it: nothing fills in after a menu opens. A server
// render is that first paint, before any effect could fetch.

import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(() => Promise.resolve(null)) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));

const app = vi.hoisted(() => ({ currentUser: { login: 'viewer' }, currentStream: null }));
vi.mock('../stores/AppStore', () => ({
  useAppStore: Object.assign((select: (s: typeof app) => unknown) => select(app), {
    getState: () => ({ ...app, addToast: () => {} }),
  }),
}));
vi.mock('../stores/chatConnectionStore', () => ({
  useChatConnectionStore: Object.assign((select: (s: { revision: number }) => unknown) => select({ revision: 0 }), {
    getState: () => ({ channels: new Map() }),
  }),
}));
// A server render reads a zustand store's INITIAL state, so the hook's view of
// the channel state is read straight from the live store here.
vi.mock('../stores/channelStateStore', async (importOriginal) => {
  const real = await importOriginal<typeof import('../stores/channelStateStore')>();
  return {
    ...real,
    useChannelState: (login: string | null | undefined) =>
      login ? (real.useChannelStateStore.getState().channels.get(login.toLowerCase()) ?? null) : null,
  };
});

import { useChannelStateStore } from '../stores/channelStateStore';
import type { ChannelRelationship, ChannelState } from '../types';
import { useChannelSocial } from './useChannelSocial';

function Probe() {
  const s = useChannelSocial({ provider: 'twitch', userId: '71092938', userLogin: 'xqc', userName: 'xQc' });
  return (
    <i
      data-following={String(s.isFollowing)}
      data-checking={String(s.checkingFollowStatus)}
      data-subscribed={String(s.isSubscribed)}
      data-history={String(s.hasSubHistory)}
      data-badge={String(s.subscriberBadgeUrl)}
    />
  );
}

const firstPaint = (): Record<string, string> => {
  const html = renderToStaticMarkup(<Probe />);
  return Object.fromEntries([...html.matchAll(/data-(\w+)="([^"]*)"/g)].map((m) => [m[1], m[2]]));
};

const watched = (relationship: ChannelRelationship | null): ChannelState => ({
  login: 'xqc',
  channel_id: '71092938',
  viewer_count: null,
  started_at: null,
  title: null,
  game_name: null,
  viewers_at: null,
  points: null,
  points_at: null,
  bits: null,
  bits_at: null,
  pinned: [],
  pinned_at: null,
  collab: null,
  shared_chat: null,
  collab_at: null,
  relationship,
  relationship_at: relationship ? 5 : null,
});

afterEach(() => useChannelStateStore.setState({ channels: new Map() }));

describe('chat menu follow and subscribe state', () => {
  it('a watched chat opens its menu with the relationship already known', () => {
    useChannelStateStore.setState({
      channels: new Map([
        [
          'xqc',
          watched({ following: true, subscribed: false, has_sub_history: true, cumulative_months: 7, badge_url: 'b9' }),
        ],
      ]),
    });
    expect(firstPaint()).toEqual({
      following: 'true',
      checking: 'false',
      subscribed: 'false',
      history: 'true',
      badge: 'b9',
    });
  });

  it('a pushed follow change is what the menu shows next', () => {
    const rel = { following: false, subscribed: true, has_sub_history: false, cumulative_months: 3, badge_url: 'b3' };
    useChannelStateStore.setState({ channels: new Map([['xqc', watched(rel)]]) });
    useChannelStateStore.getState().apply({
      section: 'relationship',
      login: 'xqc',
      relationship: { ...rel, following: true },
      at: 9,
    });
    expect(firstPaint().following).toBe('true');
    expect(firstPaint().subscribed).toBe('true');
  });

  it('a channel no chat is watching still waits for its lookup', () => {
    expect(firstPaint()).toMatchObject({ following: 'null', checking: 'true' });
  });
});
