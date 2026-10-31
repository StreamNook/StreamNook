import { beforeEach, describe, expect, it, vi } from 'vitest';

// Every set_favorite_channel call waits until the test answers it.
const mock = vi.hoisted(() => ({
  calls: [] as Array<Record<string, unknown>>,
  answers: [] as Array<(value: unknown, fail?: boolean) => void>,
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args: Record<string, unknown>) => {
    if (cmd !== 'set_favorite_channel') return Promise.resolve(undefined);
    mock.calls.push(args);
    return new Promise((resolve, reject) => {
      mock.answers.push((value, fail) => (fail ? reject(value) : resolve(value)));
    });
  },
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}), emit: vi.fn(async () => {}) }));
vi.mock('../bootPreload', () => ({ takePreloadedSettings: () => null }));

import { useAppStore } from '../stores/AppStore';
import { isFavoriteNow, resetFavoriteDoorForTests, setFavoriteChannel } from './favoriteDoor';

const flush = () => new Promise((r) => setTimeout(r, 0));
const xqc = { provider: 'twitch', userId: '7', userLogin: 'xqc' };

function push(ids: string[], rev: number) {
  useAppStore.getState().applyHomeUpdate({
    section: 'following',
    favorites: [],
    live: [],
    offline: [],
    followed_live: [],
    platforms: [],
    favorite_ids: ids,
    favorites_list: [],
    rev,
    at: 0,
  });
}

beforeEach(() => {
  mock.calls.length = 0;
  mock.answers.length = 0;
  resetFavoriteDoorForTests();
  useAppStore.setState({
    favoritePending: {},
    favoriteAlias: {},
    favoriteVersion: 0,
    followingRev: 0,
    followingApplied: false,
    following: { favorites: [], live: [], offline: [], followed_live: [], platforms: [], favorite_ids: [], favorites_list: [] },
    toasts: [],
  });
});

describe('setFavoriteChannel', () => {
  it('flips the label at once and sends presses one at a time, in order', async () => {
    const first = setFavoriteChannel(xqc, true);
    expect(isFavoriteNow(xqc)).toBe(true);
    const second = setFavoriteChannel(xqc, false);
    expect(isFavoriteNow(xqc)).toBe(false);
    await flush();
    expect(mock.calls.map((c) => c.favorite)).toEqual([true]);
    mock.answers[0]({ favorite: true, id: '7' });
    await expect(first).resolves.toBe(true);
    await flush();
    expect(mock.calls.map((c) => c.favorite)).toEqual([true, false]);
    // The older answer did not take the label back from the newer press.
    expect(isFavoriteNow(xqc)).toBe(false);
    mock.answers[1]({ favorite: false, id: '7' });
    await expect(second).resolves.toBe(false);
    expect(isFavoriteNow(xqc)).toBe(false);
    expect(useAppStore.getState().favoritePending).toEqual({});
  });

  it('keeps the label until the push agrees, then reads the push', async () => {
    const done = setFavoriteChannel(xqc, true);
    await flush();
    mock.answers[0]({ favorite: true, id: '7', rev: 1 });
    await done;
    expect(useAppStore.getState().favoritePending['id:7']?.answered).toBe(true);
    expect(isFavoriteNow(xqc)).toBe(true);
    push(['7'], 1);
    expect(useAppStore.getState().favoritePending).toEqual({});
    expect(isFavoriteNow(xqc)).toBe(true);
  });

  it('a null answer clears the press and says why', async () => {
    const done = setFavoriteChannel({ provider: 'youtube', userLogin: 'AGr94tpNVkw' }, true);
    await flush();
    mock.answers[0](null);
    await expect(done).resolves.toBeNull();
    expect(useAppStore.getState().favoritePending).toEqual({});
    expect(useAppStore.getState().toasts.map((t) => t.message)).toContain("Couldn't work out which channel this is");
  });

  it('a failed write clears the press and the next press still goes', async () => {
    const failed = setFavoriteChannel(xqc, true);
    await flush();
    mock.answers[0]('disk full', true);
    await expect(failed).resolves.toBeNull();
    expect(isFavoriteNow(xqc)).toBe(false);
    const next = setFavoriteChannel(xqc, true);
    await flush();
    expect(mock.calls).toHaveLength(2);
    mock.answers[1]({ favorite: true, id: '7' });
    await expect(next).resolves.toBe(true);
  });

  it('an answer other than wanted clears the press', async () => {
    const done = setFavoriteChannel(xqc, true);
    await flush();
    mock.answers[0]({ favorite: false, id: '7' });
    await done;
    expect(useAppStore.getState().favoritePending).toEqual({});
    expect(isFavoriteNow(xqc)).toBe(false);
  });

  it('a row Rust names by another id reads that id from then on', async () => {
    const video = { provider: 'youtube', userId: '', userLogin: 'AGr94tpNVkw' };
    const done = setFavoriteChannel(video, true);
    await flush();
    mock.answers[0]({ favorite: true, id: 'youtube:UCabc', rev: 1 });
    await done;
    const s = useAppStore.getState();
    expect(s.favoriteAlias['login:youtube:agr94tpnvkw']).toBe('id:youtube:UCabc');
    expect(isFavoriteNow(video)).toBe(true);
    push(['youtube:UCabc'], 1);
    expect(useAppStore.getState().favoritePending).toEqual({});
    expect(isFavoriteNow(video)).toBe(true);
  });

  it('labels a Twitch room without an id from the kept channel', () => {
    useAppStore.getState().applyHomeUpdate({
      section: 'following',
      favorites: [],
      live: [],
      offline: [],
      favorite_ids: ['7'],
      favorites_list: [{ id: '7', provider: 'twitch', channel: 'xqc' }],
      rev: 1,
      at: 0,
    });
    expect(isFavoriteNow({ provider: 'twitch', userLogin: 'XQC' })).toBe(true);
  });

  it('a room by login pressed twice before any push ends as last pressed, never stuck', async () => {
    const room = { provider: 'twitch', userId: '', userLogin: 'xqc' };
    const first = setFavoriteChannel(room, true);
    await flush();
    mock.answers[0]({ favorite: true, id: '7', rev: 1 });
    await first;
    expect(isFavoriteNow(room)).toBe(true);
    const second = setFavoriteChannel(room, false);
    expect(isFavoriteNow(room)).toBe(false);
    // The settled first press at the stored id is gone: it can never show again.
    expect(useAppStore.getState().favoritePending['id:7']).toBeUndefined();
    await flush();
    // The second answer lands before the first write's push.
    mock.answers[1]({ favorite: false, id: '7', rev: 2 });
    await second;
    expect(isFavoriteNow(room)).toBe(false);
    // The first write's push does not settle the second press.
    push(['7'], 1);
    expect(isFavoriteNow(room)).toBe(false);
    // The push that holds the second write settles it; the list decides.
    push([], 2);
    expect(useAppStore.getState().favoritePending).toEqual({});
    expect(isFavoriteNow(room)).toBe(false);
  });

  it('a push older than the write leaves the press; the push that holds it settles it', async () => {
    const done = setFavoriteChannel(xqc, true);
    await flush();
    mock.answers[0]({ favorite: true, id: '7', rev: 7 });
    await done;
    push([], 6);
    expect(useAppStore.getState().favoritePending['id:7']).toMatchObject({ answered: true, settlesAtRev: 7 });
    expect(isFavoriteNow(xqc)).toBe(true);
    push(['7'], 7);
    expect(useAppStore.getState().favoritePending).toEqual({});
    expect(isFavoriteNow(xqc)).toBe(true);
  });

  it('the push that holds the write settles the press even when another writer changed it since', async () => {
    const done = setFavoriteChannel(xqc, true);
    await flush();
    mock.answers[0]({ favorite: true, id: '7', rev: 3 });
    await done;
    // `/unfavorite` in another window landed before this rebuild: the list says no.
    push([], 3);
    expect(useAppStore.getState().favoritePending).toEqual({});
    expect(isFavoriteNow(xqc)).toBe(false);
  });

  it('an answer whose revision this window already holds settles at once', async () => {
    push(['7'], 2);
    const done = setFavoriteChannel(xqc, true);
    await flush();
    mock.answers[0]({ favorite: true, id: '7', rev: 2 });
    await done;
    expect(useAppStore.getState().favoritePending).toEqual({});
    expect(isFavoriteNow(xqc)).toBe(true);
  });

  it('sends the row fields Rust names the channel from', async () => {
    void setFavoriteChannel({ provider: 'kick', userId: '55', userLogin: 'kick:xqc', userName: 'xQc', avatar: 'a.png' }, true);
    await flush();
    expect(mock.calls[0]).toEqual({
      provider: 'kick',
      userId: '55',
      userLogin: 'kick:xqc',
      userName: 'xQc',
      avatar: 'a.png',
      favorite: true,
    });
  });
});
