import { describe, it, expect, vi } from 'vitest';

// channelSearch reaches the app store (Twitch follows) and the follows store
// (provider follows) at import time, and AppStore touches `window` at module
// scope. None of that is under test here: these cases are the pure key-space
// and parsing helpers.
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(null) }));
vi.mock('../../stores/AppStore', () => ({
  useAppStore: Object.assign(vi.fn(), { getState: () => ({ addToast: vi.fn() }), setState: vi.fn(), subscribe: vi.fn() }),
}));
vi.mock('../../stores/followsStore', () => ({
  useFollowsStore: Object.assign(vi.fn(), { getState: () => ({ liveByKey: {} }), setState: vi.fn(), subscribe: vi.fn() }),
}));

import {
  rankResults,
  itemKey,
  filterFollowing,
  filterSearch,
  parseTypedChannel,
  streamToItem,
  resultToItem,
  type ChannelItem,
} from './channelSearch';
import { makeKey } from '../../utils/providerKey';

function item(over: Partial<ChannelItem> & { login: string }): ChannelItem {
  return { id: over.login, displayName: over.login, isLive: true, source: 'search', ...over };
}

describe('itemKey', () => {
  it('treats an absent provider as Twitch', () => {
    expect(itemKey({ login: 'xqc' })).toBe('twitch:xqc');
  });

  it('keeps the same login on two platforms apart', () => {
    expect(itemKey({ login: 'xqc', provider: 'kick' })).not.toBe(itemKey({ login: 'xqc' }));
  });

  it('preserves YouTube casing', () => {
    expect(itemKey({ login: 'AGr94tpNVkw', provider: 'youtube' })).toBe('youtube:AGr94tpNVkw');
  });
});

// The headline Phase 5 regression. Before this, the picker collapsed a channel
// to a bare lowercased login, so a Twitch tile in the grid DELETED the Kick row
// of the same name from the search results: the Kick channel was unreachable
// precisely because a Twitch one was on screen.
describe('filterFollowing', () => {
  const exclude = new Set([makeKey('twitch', 'xqc')]);

  it('excludes the Twitch channel that is already in the grid', () => {
    expect(filterFollowing([item({ login: 'xqc' })], exclude, '')).toHaveLength(0);
  });

  it('does NOT exclude the same login on another platform', () => {
    const kept = filterFollowing([item({ login: 'xqc', provider: 'kick' })], exclude, '');
    expect(kept).toHaveLength(1);
    expect(kept[0].provider).toBe('kick');
  });

  it('does not fold YouTube casing when excluding', () => {
    const yt = item({ login: 'AGr94tpNVkw', provider: 'youtube' });
    const lowered = new Set([makeKey('twitch', 'agr94tpnvkw')]);
    expect(filterFollowing([yt], lowered, '')).toHaveLength(1);
  });

  // These are substring display filters in an OR chain, not key comparisons.
  // Folding them into the key space would break searching by name or category.
  it('still matches on login, display name and category', () => {
    const rows = [
      item({ login: 'aaa', displayName: 'Cool Person' }),
      item({ login: 'bbb', displayName: 'bbb', gameName: 'Chess' }),
      item({ login: 'ccc', displayName: 'ccc' }),
    ];
    expect(filterFollowing(rows, new Set(), 'cool')).toHaveLength(1);
    expect(filterFollowing(rows, new Set(), 'chess')).toHaveLength(1);
    expect(filterFollowing(rows, new Set(), 'ccc')).toHaveLength(1);
  });
});

describe('filterSearch', () => {
  it('drops a search hit already shown as a live follow', () => {
    const following = new Set([itemKey({ login: 'xqc' })]);
    expect(filterSearch([item({ login: 'xqc' })], new Set(), following)).toHaveLength(0);
  });

  it('keeps a Kick hit when the Twitch channel of that name is a follow', () => {
    const following = new Set([itemKey({ login: 'xqc' })]);
    expect(filterSearch([item({ login: 'xqc', provider: 'kick' })], new Set(), following)).toHaveLength(1);
  });

  // Two platforms answering one query can return the same login. Without a
  // composite dedupe the second row collides with the first in React's key space.
  it('dedupes within a result set on the composite key, not the login', () => {
    const rows = [
      item({ login: 'shared' }),
      item({ login: 'shared', provider: 'kick' }),
      item({ login: 'shared' }),
    ];
    const kept = filterSearch(rows, new Set(), new Set());
    expect(kept).toHaveLength(2);
    expect(new Set(kept.map(itemKey)).size).toBe(2);
  });

  it('drops rows with no login at all', () => {
    expect(filterSearch([item({ login: '' })], new Set(), new Set())).toHaveLength(0);
  });
});

// Typed text carries no platform. Accepting the app's own provider:channel form
// is what stops `kick:xqc` becoming a Twitch channel literally named "kick:xqc",
// which in the preset editor persisted to disk permanently.
describe('parseTypedChannel', () => {
  it('reads a bare name as Twitch', () => {
    expect(parseTypedChannel('xqc')).toEqual({ provider: 'twitch', channel: 'xqc' });
  });

  it('reads the provider form', () => {
    expect(parseTypedChannel('kick:somebody')).toEqual({ provider: 'kick', channel: 'somebody' });
  });

  it('preserves YouTube casing', () => {
    expect(parseTypedChannel('youtube:AGr94tpNVkw')?.channel).toBe('AGr94tpNVkw');
  });

  // parseKey alone does NOT reject this: it reads an unknown prefix as a legacy
  // Twitch login and returns the whole string, colon included. Rejecting is the
  // point of having a separate helper.
  it('REJECTS an unrecognised prefix rather than guessing', () => {
    expect(parseTypedChannel('weird:name')).toBeNull();
  });

  it('rejects empty and whitespace-only input, and a provider with no channel', () => {
    expect(parseTypedChannel('')).toBeNull();
    expect(parseTypedChannel('   ')).toBeNull();
    expect(parseTypedChannel('kick:')).toBeNull();
  });

  it('trims surrounding whitespace', () => {
    expect(parseTypedChannel('  xqc  ')).toEqual({ provider: 'twitch', channel: 'xqc' });
  });
});

describe('row construction', () => {
  it('leaves provider absent for Twitch and sets it otherwise', () => {
    expect(streamToItem({ user_id: '1', user_login: 'a', user_name: 'A' } as never).provider).toBeUndefined();
    expect(streamToItem({ user_id: '1', user_login: 'a', provider: 'kick' } as never).provider).toBe('kick');
  });

  it('tags a search row with the platform that answered', () => {
    expect(resultToItem({ user_login: 'a' } as never, 'kick').provider).toBe('kick');
    expect(resultToItem({ user_login: 'a' } as never).provider).toBeUndefined();
  });

  // A provider row's user_id is #[serde(default)] and can be an empty string.
  // Falling back to the login keeps the row addressable and its React key unique.
  it('falls back to the login when the platform sent no id', () => {
    expect(resultToItem({ user_login: 'somebody', user_id: '' } as never, 'kick').id).toBe('somebody');
  });

  it('trusts a follow row that reports itself offline', () => {
    expect(streamToItem({ user_login: 'a', is_live: false } as never).isLive).toBe(false);
    expect(streamToItem({ user_login: 'a' } as never).isLive).toBe(true);
  });
});

// The picker is multi-platform only if its other-platform hits are VISIBLE.
// Twitch matches fuzzily and fills the list; Kick and YouTube return one or two.
// Appending them put a real YouTube hit at position NINE, where a reviewer with
// devtools open reasonably concluded the picker searched Twitch only.
describe('rankResults', () => {
  const tw = (n: number) =>
    Array.from({ length: n }, (_, i) => item({ login: `tw${i}`, source: 'search' }));

  it('surfaces a lone provider hit above the fold', () => {
    const out = rankResults(tw(8), [item({ login: 'yt', provider: 'youtube' })]);
    const at = out.findIndex((i) => i.provider === 'youtube');
    expect(at).toBeGreaterThanOrEqual(0);
    expect(at).toBeLessThan(5);
  });

  it('keeps every result, dropping none', () => {
    const out = rankResults(tw(8), [item({ login: 'k', provider: 'kick' })]);
    expect(out).toHaveLength(9);
    expect(new Set(out.map(itemKey)).size).toBe(9);
  });

  it('leaves Twitch leading, since it is the common case', () => {
    const out = rankResults(tw(8), [item({ login: 'k', provider: 'kick' })]);
    expect(out[0].provider).toBeUndefined();
  });

  it('changes nothing when no other platform matched', () => {
    const only = tw(8);
    expect(rankResults(only, [])).toEqual(only);
  });

  it('handles fewer Twitch results than the lead without dropping or padding', () => {
    const out = rankResults(tw(2), [item({ login: 'k', provider: 'kick' })]);
    expect(out.map((i) => i.login)).toEqual(['tw0', 'tw1', 'k']);
  });
})
