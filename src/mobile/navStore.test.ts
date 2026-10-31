// What takes Back before the full player shrinks (SN-30): the list the watch
// layer's Back claim reads must match what handleBack really does first.
import { describe, expect, it, vi } from 'vitest';

vi.hoisted(() => {
  const g = globalThis as Record<string, unknown>;
  g.window ??= { dispatchEvent: () => true, addEventListener: () => {}, removeEventListener: () => {} };
  g.CustomEvent ??= class {
    constructor(
      public type: string,
      public init?: unknown,
    ) {}
  };
});
vi.mock('../stores/AppStore', () => ({
  useAppStore: { getState: () => ({ streamUrl: 'https://example.test/live.m3u8', isLoading: false }) },
}));
vi.mock('./phonePrefs', () => ({ startTab: () => 'following' }));

import { backTakenBeforePlayer, useMobileNavStore } from './navStore';

const base = {
  sheetStack: [] as string[],
  songsOpen: false,
  songsFocus: null,
  whispersOpen: false,
  mentionsOpen: false,
  settingsView: null as string | null,
  cosmeticsOpen: false,
  browseCategory: null,
  playerMode: 'full' as const,
};

describe('Back before the player', () => {
  it.each([
    { sheetStack: ['quality'] },
    { songsOpen: true },
    { whispersOpen: true },
    { mentionsOpen: true },
    { settingsView: 'player' },
    { cosmeticsOpen: true },
  ])('%o takes Back before the player shrinks', (open) => {
    useMobileNavStore.setState({ ...base, ...open });
    expect(backTakenBeforePlayer(useMobileNavStore.getState())).toBe(true);
    expect(useMobileNavStore.getState().handleBack()).toBe(true);
    expect(useMobileNavStore.getState().playerMode).toBe('full');
  });

  it('with nothing open, Back shrinks the full player', () => {
    useMobileNavStore.setState({ ...base });
    expect(backTakenBeforePlayer(useMobileNavStore.getState())).toBe(false);
    expect(useMobileNavStore.getState().handleBack()).toBe(true);
    expect(useMobileNavStore.getState().playerMode).toBe('mini');
  });
});

describe('Back on Chats', () => {
  it('leaves Chats before a category page left open under it (SN-37)', () => {
    useMobileNavStore.setState({
      ...base,
      playerMode: 'mini',
      activeTab: 'chats',
      chatsReturnTab: 'browse',
      browseCategory: { id: '509658', name: 'Just Chatting' } as never,
    });
    expect(useMobileNavStore.getState().handleBack()).toBe(true);
    expect(useMobileNavStore.getState().activeTab).toBe('browse');
    expect(useMobileNavStore.getState().browseCategory).not.toBeNull();
  });
});
