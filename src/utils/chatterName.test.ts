import { describe, expect, it, vi } from 'vitest';

vi.mock('../stores/AppStore', () => ({ useAppStore: { getState: () => ({ settings: {} }) } }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

import { overrideKey, rowBaseName, rowOverrideKey, rowShownName, type NamedRow } from './chatterName';
import { chatterId, cosmeticsKey } from './chatterIdentity';
import { getDisplayedName } from './userChatOverrides';
import type { UserChatOverride } from '../types';

// Rows as the page receives them from Rust for one localized chatter: a plain
// message, and a watch-streak notice whose `display-name` tag is the login.
// Rust stamps the same shown name on both (services::chatter_name tests).
const SHOWN = 'もちパンダ (mochi_panda)';
const privmsg: NamedRow = {
  username: 'mochi_panda',
  displayName: 'もちパンダ',
  tags: new Map([['display-name', 'もちパンダ'], ['user-id', '700700']]),
  metadata: { shown_name: SHOWN },
};
const watchStreak: NamedRow = {
  username: 'mochi_panda',
  displayName: 'もちパンダ',
  tags: new Map([
    ['display-name', 'mochi_panda'],
    ['user-id', '700700'],
    ['msg-id', 'viewermilestone'],
    ['msg-param-category', 'watch-streak'],
  ]),
  metadata: { shown_name: SHOWN },
};

describe('rowShownName', () => {
  it('names a localized chatter the same on a message and a watch streak', () => {
    expect(rowShownName(privmsg, '700700', {})).toBe(SHOWN);
    expect(rowShownName(watchStreak, '700700', {})).toBe(SHOWN);
  });

  it('applies a nickname override on both row types', () => {
    const overrides = { '700700': { user_id: '700700', nickname: 'Cake' } };
    expect(rowShownName(privmsg, '700700', overrides)).toBe('Cake');
    expect(rowShownName(watchStreak, '700700', overrides)).toBe('Cake');
  });

  it('ignores a blank nickname', () => {
    expect(rowShownName(privmsg, '700700', { '700700': { user_id: '700700', nickname: '  ' } })).toBe(SHOWN);
  });

  it('shows a case-only display name as Rust stamped it', () => {
    const row: NamedRow = {
      username: 'mochi',
      tags: new Map([['display-name', 'Mochi']]),
      metadata: { shown_name: 'Mochi' },
    };
    expect(rowShownName(row, '1', undefined)).toBe('Mochi');
  });

  it('falls back to the display name on a row Rust never stamped', () => {
    expect(rowBaseName({ username: 'mochi_panda', tags: new Map([['display-name', 'もちパンダ']]) })).toBe('もちパンダ');
    expect(rowBaseName({ username: 'mochi_panda', tags: new Map() })).toBe('mochi_panda');
  });
});

// One localized chatter, every save path and every row type. The profile card is
// the only place a nickname is saved; it is opened from a click payload whose id
// differs per surface, and saves under overrideKey(id, provider). Every renderer
// must read the same key back.
describe('a saved nickname for a localized chatter', () => {
  const ID = '700700';
  const twitchRow = (extra: [string, string][]) => ({
    username: 'mochi_panda',
    displayName: 'もちパンダ',
    tags: new Map<string, string>([['user-id', ID], ...extra]),
    metadata: { shown_name: SHOWN },
  });
  const plain = twitchRow([['display-name', 'もちパンダ']]);
  const streak = twitchRow([
    ['display-name', 'mochi_panda'],
    ['login', 'mochi_panda'],
    ['msg-id', 'viewermilestone'],
    ['msg-param-category', 'watch-streak'],
  ]);
  const sub = twitchRow([
    ['display-name', 'もちパンダ'],
    ['login', 'mochi_panda'],
    ['msg-id', 'resub'],
  ]);

  // The id each surface hands the card when this chatter is clicked.
  const savePaths: Record<string, string | undefined> = {
    'plain row': chatterId(plain),
    'watch-streak card': chatterId(streak),
    'sub card': chatterId(sub),
    'viewers list (Helix user_id)': ID,
    'mention (chatter store key)': cosmeticsKey(ID, 'twitch'),
    'pinned card (pin sender_id)': ID,
  };

  for (const [path, clickedId] of Object.entries(savePaths)) {
    it(`saved from the ${path} is found on every row type`, () => {
      const key = overrideKey(clickedId, 'twitch');
      expect(key).toBe(ID);
      const overrides: Record<string, UserChatOverride> = {
        [key]: { user_id: key, username: 'mochi_panda', nickname: 'Cake' },
      };
      expect(rowShownName(plain, rowOverrideKey(plain), overrides)).toBe('Cake');
      expect(rowShownName(streak, rowOverrideKey(streak), overrides)).toBe('Cake');
      expect(rowShownName(sub, rowOverrideKey(sub), overrides)).toBe('Cake');
      // Reply header: the parent is addressed by reply-parent-user-id.
      expect(getDisplayedName(overrideKey(ID, 'twitch'), '親 (mochi_panda)', overrides)).toBe('Cake');
      // Pinned card: useChatterLook keys by cosmeticsKey(sender_id, provider).
      expect(getDisplayedName(cosmeticsKey(ID, 'twitch'), SHOWN, overrides)).toBe('Cake');
    });
  }

  it('saves and reads a Kick chatter under the namespaced key', () => {
    const kickRow = {
      username: 'stone916',
      provider: 'kick' as const,
      providerUserId: '676',
      tags: new Map<string, string>(),
    };
    expect(overrideKey('676', 'kick')).toBe('kick:676');
    expect(rowOverrideKey(kickRow)).toBe('kick:676');
  });

  it('has no key without an id, so the card offers no editor', () => {
    expect(overrideKey('', 'twitch')).toBe('');
    expect(overrideKey(undefined, 'kick')).toBe('');
  });
});
