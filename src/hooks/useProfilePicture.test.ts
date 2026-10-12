// Run with: npm test
//
// The "Profile pictures" source as the hooks apply it. The hooks are called as
// plain functions with the store and the picture memo stubbed, so what is
// checked is the wiring: on the Twitch source the picture memo is asked about
// no id at all, so nothing reaches Rust or 7TV.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { SevenTvPicture } from '../services/sevenTvPictures';

const state = vi.hoisted(() => ({
  settings: { cosmetics: {} as Record<string, unknown> },
  asked: [] as Array<{ id: string | null | undefined; ask: boolean }>,
  picture: null as SevenTvPicture | null,
}));

vi.mock('../stores/AppStore', () => ({
  useAppStore: (select: (s: typeof state) => unknown) => select(state),
}));

vi.mock('../services/sevenTvPictures', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../services/sevenTvPictures')>()),
  useSevenTvPicture: (id: string | null | undefined, ask = true) => {
    state.asked.push({ id, ask });
    return id ? state.picture : null;
  },
}));

import { useProfilePicture, useProfilePictureSource } from './useProfilePicture';

const base = 'https://cdn.7tv.app/user/u/profile-picture/p';
const moving: SevenTvPicture = {
  id: 'p',
  still: [1, 2, 3, 4].map((scale) => ({ scale, url: `${base}/${scale}x_static.webp` })),
  animated: [1, 2, 3, 4].map((scale) => ({ scale, url: `${base}/${scale}x.webp` })),
};

beforeEach(() => {
  state.settings = { cosmetics: {} };
  state.asked = [];
  state.picture = moving;
});

describe('useProfilePictureSource', () => {
  it('reads 7TV when unset or unknown, and Twitch only when chosen', () => {
    expect(useProfilePictureSource()).toBe('seventv');
    state.settings.cosmetics.profile_picture_source = 'bogus';
    expect(useProfilePictureSource()).toBe('seventv');
    state.settings.cosmetics.profile_picture_source = 'twitch';
    expect(useProfilePictureSource()).toBe('twitch');
  });
});

describe('useProfilePicture', () => {
  it('looks a person up on the 7TV source', () => {
    expect(useProfilePicture('71092938')).toBe(moving);
    expect(state.asked).toEqual([{ id: '71092938', ask: true }]);
  });

  it('asks about no id at all on the Twitch source', () => {
    state.settings.cosmetics.profile_picture_source = 'twitch';
    expect(useProfilePicture('71092938')).toBeNull();
    expect(useProfilePicture('kick:26298')).toBeNull();
    expect(useProfilePicture('youtube:UCX6OQ3DkcsbYNE6H8uQQuVA')).toBeNull();
    expect(state.asked).toHaveLength(3);
    expect(state.asked.every((a) => a.id === null)).toBe(true);
  });

  it('never asks about an id 7TV cannot know', () => {
    useProfilePicture('tiktok:123');
    useProfilePicture('some_login');
    expect(state.asked.map((a) => a.id)).toEqual([null, null]);
  });

  it('reads a primed id as it is when it only reads (the settings sample)', () => {
    useProfilePicture('streamnook:profile-picture-sample', false);
    expect(state.asked).toEqual([{ id: 'streamnook:profile-picture-sample', ask: false }]);
  });

  it('lets the settings preview draw either source whatever is saved', () => {
    state.settings.cosmetics.profile_picture_source = 'twitch';
    expect(useProfilePicture('71092938', true, 'seventv')).toBe(moving);
    state.settings.cosmetics.profile_picture_source = 'seventv';
    expect(useProfilePicture('71092938', true, 'twitch')).toBeNull();
  });
});
