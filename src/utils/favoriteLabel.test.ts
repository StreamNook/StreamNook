import { describe, expect, it } from 'vitest';
import {
  favoriteKeyOf,
  favoritesDiffer,
  labelFavorite,
  settlePending,
  type FavoriteLabelState,
} from './favoriteLabel';

const empty: FavoriteLabelState = { pending: {}, alias: {}, ids: [], list: [] };

describe('favoriteKeyOf', () => {
  it('tags a named channel by its favorite id and an unnamed one by login', () => {
    expect(favoriteKeyOf({ provider: 'twitch', userId: '7', userLogin: 'xqc' })).toBe('id:7');
    expect(favoriteKeyOf({ provider: 'kick', userLogin: 'XQC' })).toBe('id:kick:xqc');
    expect(favoriteKeyOf({ provider: 'kick', userLogin: 'kick:xqc' })).toBe('id:kick:xqc');
    expect(favoriteKeyOf({ provider: 'twitch', userId: '', userLogin: ' XQC ' })).toBe('login:twitch:xqc');
    expect(favoriteKeyOf({ provider: 'youtube', userId: '', userLogin: 'AGr94tpNVkw' })).toBe(
      'login:youtube:agr94tpnvkw',
    );
  });
});

describe('labelFavorite', () => {
  it('reads Rust pushed ids for a named channel', () => {
    const state = { ...empty, ids: ['7'] };
    expect(labelFavorite({ provider: 'twitch', userId: '7', userLogin: 'xqc' }, state)).toBe(true);
    expect(labelFavorite({ provider: 'twitch', userId: '8', userLogin: 'other' }, state)).toBe(false);
  });

  it('names a Twitch room without an id from the kept channel', () => {
    const state: FavoriteLabelState = {
      ...empty,
      ids: ['7'],
      list: [{ id: '7', provider: 'twitch', channel: 'xqc' }],
    };
    expect(labelFavorite({ provider: 'twitch', userLogin: 'XQC' }, state)).toBe(true);
    expect(labelFavorite({ provider: 'kick', userLogin: 'xqc' }, state)).toBe(false);
    // A kept channel whose id is no longer listed is not a favorite.
    expect(labelFavorite({ provider: 'twitch', userLogin: 'xqc' }, { ...state, ids: [] })).toBe(false);
  });

  it('a press on its way wins over the pushed ids', () => {
    const state: FavoriteLabelState = {
      ...empty,
      ids: ['7'],
      pending: { 'id:7': { wanted: false, seq: 1, answered: false, settlesAtRev: 0 } },
    };
    expect(labelFavorite({ provider: 'twitch', userId: '7', userLogin: 'xqc' }, state)).toBe(false);
  });

  it('a row known by login reads its alias', () => {
    const state: FavoriteLabelState = {
      ...empty,
      ids: ['youtube:UCabc'],
      alias: { 'login:youtube:agr94tpnvkw': 'id:youtube:UCabc' },
    };
    expect(labelFavorite({ provider: 'youtube', userLogin: 'AGr94tpNVkw' }, state)).toBe(true);
  });
});

describe('settlePending', () => {
  it('drops an answered press once a list at or past its revision is applied, and keeps the rest', () => {
    const pending = {
      'id:7': { wanted: true, seq: 1, answered: true, settlesAtRev: 4 },
      'login:twitch:xqc': { wanted: true, seq: 1, answered: true, settlesAtRev: 5 },
      'id:8': { wanted: true, seq: 1, answered: false, settlesAtRev: 0 },
    };
    expect(Object.keys(settlePending(pending, 3))).toEqual(['id:7', 'login:twitch:xqc', 'id:8']);
    expect(Object.keys(settlePending(pending, 4))).toEqual(['login:twitch:xqc', 'id:8']);
    expect(Object.keys(settlePending(pending, 5))).toEqual(['id:8']);
  });

  it('returns the same object when nothing settles', () => {
    const pending = { 'id:7': { wanted: true, seq: 1, answered: false, settlesAtRev: 0 } };
    expect(settlePending(pending, 9)).toBe(pending);
  });
});

describe('favoritesDiffer', () => {
  it('notices a changed id list or kept channel', () => {
    const a = { ids: ['7'], list: [{ id: '7', provider: 'twitch', channel: 'xqc' }] };
    expect(favoritesDiffer(a, { ...a })).toBe(false);
    expect(favoritesDiffer(a, { ...a, ids: [] })).toBe(true);
    expect(favoritesDiffer(a, { ...a, list: [] })).toBe(true);
  });
});
