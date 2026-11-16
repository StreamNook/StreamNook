// The phone's link actions: a watch link's closeLinkSheet, a category link's
// openCategoryLink and its fill, and what each link clears that another set.
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { dispatched, closeSpotlight, invoke } = vi.hoisted(() => {
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
  return { dispatched, closeSpotlight: vi.fn(), invoke: vi.fn() };
});
vi.mock('../stores/AppStore', () => ({
  useAppStore: { getState: () => ({ streamUrl: null, isLoading: false }) },
}));
vi.mock('../stores/badgeSpotlightStore', () => ({
  useBadgeSpotlightStore: { getState: () => ({ close: closeSpotlight }) },
}));
vi.mock('./phonePrefs', () => ({ startTab: () => 'following' }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

import { useMobileNavStore } from './navStore';
import { getLinkedCategory, resetLinkedCategories, resolveLinkedCategory } from '../utils/linkedCategory';

const nav = () => useMobileNavStore.getState();

const OPEN_LAYERS = {
  settingsView: 'player',
  settingsTarget: { section: 'Playback' },
  settingsReturn: 'changed',
  cosmeticsOpen: true,
  mentionsOpen: true,
  whisperKey: 'w',
  songsOpen: true,
  songsFocus: 's',
};
const LAYERS_CLOSED = {
  settingsView: null,
  settingsTarget: null,
  settingsReturn: null,
  cosmeticsOpen: false,
  mentionsOpen: false,
  whisperKey: null,
  songsOpen: false,
  songsFocus: null,
};
const FOUND = { state: 'found', id: '509658', name: 'Just Chatting', box_art_url: 'art', viewer_count: null };

beforeEach(() => {
  dispatched.length = 0;
  closeSpotlight.mockClear();
  invoke.mockReset();
  resetLinkedCategories();
  useMobileNavStore.setState({
    activeTab: 'following',
    sheetStack: [],
    ...LAYERS_CLOSED,
    playerMode: 'full',
    badgeLinkKey: null,
    browseCategory: null,
    linkSheetSeq: 0,
  });
});

describe('closeLinkSheet', () => {
  // Fails if a watch link leaves a sheet, a layer or the badge key over the stream.
  it('dismisses every sheet top first, closes the spotlight and every layer, clears the key and bumps the counter', () => {
    useMobileNavStore.setState({ ...OPEN_LAYERS, sheetStack: ['a', 'b'], badgeLinkKey: 'clover/3', linkSheetSeq: 4 });
    nav().closeLinkSheet();
    expect(dispatched).toEqual([
      { type: 'sn:dismiss-sheet', detail: 'b' },
      { type: 'sn:dismiss-sheet', detail: 'a' },
    ]);
    expect(closeSpotlight).toHaveBeenCalledTimes(1);
    expect(nav()).toMatchObject(LAYERS_CLOSED);
    expect(nav().badgeLinkKey).toBeNull();
    expect(nav().linkSheetSeq).toBe(5);
  });

  // Fails if the watch link shrinks the player, changes the tab or closes the category the stream plays over.
  it('leaves the player, the tab and an open category', () => {
    const category = { id: '1', name: 'Alpha', box_art_url: '' };
    useMobileNavStore.setState({ activeTab: 'browse', browseCategory: category });
    nav().closeLinkSheet();
    expect(nav().playerMode).toBe('full');
    expect(nav().activeTab).toBe('browse');
    expect(nav().browseCategory).toBe(category);
  });
});

describe('openCategoryLink', () => {
  // Fails if any of it waits for a promise, or leaves Chats, a sheet, a layer or the badge key.
  it('lands on Browse on a nameless copy, shrinks the player, closes everything and bumps the counter', () => {
    useMobileNavStore.setState({ ...OPEN_LAYERS, sheetStack: ['a'], activeTab: 'chats', badgeLinkKey: 'clover/3' });
    nav().openCategoryLink('509658');
    expect(nav().browseCategory).toEqual({ id: '509658', name: '', box_art_url: '' });
    expect(nav().activeTab).toBe('browse');
    expect(nav().playerMode).toBe('mini');
    expect(dispatched).toEqual([{ type: 'sn:dismiss-sheet', detail: 'a' }]);
    expect(closeSpotlight).toHaveBeenCalledTimes(1);
    expect(nav()).toMatchObject(LAYERS_CLOSED);
    expect(nav().badgeLinkKey).toBeNull();
    expect(nav().linkSheetSeq).toBe(1);
  });

  // Fails if a new link keeps an old failure, so the screen never asks again.
  it("drops the id's failed record", async () => {
    invoke.mockRejectedValueOnce(new Error('offline'));
    await resolveLinkedCategory('509658');
    expect(getLinkedCategory('509658')?.state).toBe('failed');
    nav().openCategoryLink('509658');
    expect(getLinkedCategory('509658')).toBeUndefined();
  });

  // Fails if a link to the category already shown blanks its name.
  it('keeps the name of the category already shown', () => {
    const shown = { id: '509658', name: 'Just Chatting', box_art_url: 'art' };
    useMobileNavStore.setState({ browseCategory: shown });
    nav().openCategoryLink('509658');
    expect(nav().browseCategory).toBe(shown);
  });

  // Fails if a known name is thrown away for the loading header.
  it('names the copy at once from a found record', async () => {
    invoke.mockResolvedValueOnce(FOUND);
    await resolveLinkedCategory('509658');
    nav().openCategoryLink('509658');
    expect(nav().browseCategory).toEqual({ id: '509658', name: 'Just Chatting', box_art_url: 'art' });
  });

  // Fails if another id inherits the shown category's name.
  it('opens another id nameless, even over a named category', () => {
    useMobileNavStore.setState({ browseCategory: { id: '1', name: 'Alpha', box_art_url: '' } });
    nav().openCategoryLink('509658');
    expect(nav().browseCategory).toEqual({ id: '509658', name: '', box_art_url: '' });
  });
});

describe('links clear what other links set', () => {
  // Fails if a badge key outlives a later category link (Rewards would open it on its next mount).
  it('a category link after a badge link clears the badge key', () => {
    nav().openBadgeLink('a/1');
    nav().openCategoryLink('509658');
    expect(nav().badgeLinkKey).toBeNull();
  });

  // Fails if the category layer stays over Rewards after a badge link.
  it('a badge link after a category link closes the category', () => {
    nav().openCategoryLink('509658');
    nav().openBadgeLink('a/1');
    expect(nav().browseCategory).toBeNull();
    expect(nav().activeTab).toBe('rewards');
  });

  it('a drop campaign jump closes the category too', () => {
    useMobileNavStore.setState({ browseCategory: { id: '1', name: 'Alpha', box_art_url: '' } });
    nav().openDropCampaign('c1');
    expect(nav().browseCategory).toBeNull();
  });
});

describe('fillBrowseCategory', () => {
  const found = { id: '509658', name: 'Just Chatting', box_art_url: 'art' };

  it('fills the nameless copy of the same id', () => {
    nav().openCategoryLink('509658');
    nav().fillBrowseCategory('509658', found);
    expect(nav().browseCategory).toEqual(found);
  });

  // Fails if an answer lands on another category, overwrites a name, or reopens a closed screen.
  it('leaves another id, a named copy and no copy alone', () => {
    const other = { id: '1', name: '', box_art_url: '' };
    useMobileNavStore.setState({ browseCategory: other });
    nav().fillBrowseCategory('509658', found);
    expect(nav().browseCategory).toBe(other);
    const named = { id: '509658', name: 'Already', box_art_url: '' };
    useMobileNavStore.setState({ browseCategory: named });
    nav().fillBrowseCategory('509658', found);
    expect(nav().browseCategory).toBe(named);
    useMobileNavStore.setState({ browseCategory: null });
    nav().fillBrowseCategory('509658', found);
    expect(nav().browseCategory).toBeNull();
  });
});
