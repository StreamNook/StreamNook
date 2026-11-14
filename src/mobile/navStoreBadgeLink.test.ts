// The phone's badge link: openBadgeLink closes the layers and every pushed sheet
// the way they close themselves, lands on Rewards with the key, and
// clearBadgeLink keeps a newer key.
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { dispatched, closeSpotlight } = vi.hoisted(() => {
  const dispatched: Array<{ type: string; detail: unknown }> = [];
  const g = globalThis as Record<string, unknown>;
  g.CustomEvent = class {
    constructor(
      public type: string,
      init?: { detail?: unknown },
    ) {
      this.detail = init?.detail;
    }
    detail: unknown;
  };
  g.window = {
    dispatchEvent: (e: { type: string; detail: unknown }) => {
      dispatched.push({ type: e.type, detail: e.detail });
      return true;
    },
    addEventListener: () => {},
    removeEventListener: () => {},
  };
  return { dispatched, closeSpotlight: vi.fn() };
});
vi.mock('../stores/AppStore', () => ({
  useAppStore: { getState: () => ({ streamUrl: null, isLoading: false }) },
}));
vi.mock('../stores/badgeSpotlightStore', () => ({
  useBadgeSpotlightStore: { getState: () => ({ close: closeSpotlight }) },
}));
vi.mock('./phonePrefs', () => ({ startTab: () => 'following' }));

import { useMobileNavStore } from './navStore';

const nav = () => useMobileNavStore.getState();

beforeEach(() => {
  dispatched.length = 0;
  closeSpotlight.mockClear();
  useMobileNavStore.setState({
    activeTab: 'following',
    sheetStack: [],
    settingsView: null,
    settingsReturn: null,
    cosmeticsOpen: false,
    mentionsOpen: false,
    whisperKey: null,
    songsOpen: false,
    songsFocus: null,
    playerMode: 'full',
    badgeLinkKey: null,
  });
});

describe('openBadgeLink', () => {
  it('shrinks the player, lands on Rewards and sets the key', () => {
    nav().openBadgeLink('clover/2');
    expect(nav().playerMode).toBe('mini');
    expect(nav().activeTab).toBe('rewards');
    expect(nav().badgeLinkKey).toBe('clover/2');
  });

  it('closes the settings, cosmetics, mentions and songs layers', () => {
    useMobileNavStore.setState({
      settingsView: 'player',
      settingsReturn: 'x',
      cosmeticsOpen: true,
      mentionsOpen: true,
      whisperKey: 'w',
      songsOpen: true,
      songsFocus: 's',
    });
    nav().openBadgeLink('clover/1');
    expect(nav().settingsView).toBeNull();
    expect(nav().settingsReturn).toBeNull();
    expect(nav().cosmeticsOpen).toBe(false);
    expect(nav().mentionsOpen).toBe(false);
    expect(nav().whisperKey).toBeNull();
    expect(nav().songsOpen).toBe(false);
    expect(nav().songsFocus).toBeNull();
  });

  it('dismisses every pushed sheet, top first, from a snapshot', () => {
    useMobileNavStore.setState({ sheetStack: ['a', 'b', 'c'] });
    nav().openBadgeLink('clover/1');
    expect(dispatched).toEqual([
      { type: 'sn:dismiss-sheet', detail: 'c' },
      { type: 'sn:dismiss-sheet', detail: 'b' },
      { type: 'sn:dismiss-sheet', detail: 'a' },
    ]);
  });

  it('never steps back one sheet at a time (no sn:close-sheet)', () => {
    useMobileNavStore.setState({ sheetStack: ['a'] });
    nav().openBadgeLink('clover/1');
    expect(dispatched.some((e) => e.type === 'sn:close-sheet')).toBe(false);
  });

  it('closes a chat badge sheet still waiting on Rust', () => {
    nav().openBadgeLink('clover/1');
    expect(closeSpotlight).toHaveBeenCalledTimes(1);
  });
});

describe('clearBadgeLink', () => {
  it('nulls the key while it still matches', () => {
    nav().openBadgeLink('a/1');
    nav().clearBadgeLink('a/1');
    expect(nav().badgeLinkKey).toBeNull();
  });

  it('keeps a newer link key', () => {
    nav().openBadgeLink('a/1');
    nav().openBadgeLink('b/1');
    nav().clearBadgeLink('a/1');
    expect(nav().badgeLinkKey).toBe('b/1');
  });
});
