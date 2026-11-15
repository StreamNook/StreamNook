import { beforeEach, describe, expect, it, vi } from 'vitest';

// A badge link opens Badges on the Twitch tab so Back from the detail lands on
// its list, and nothing in the store waits to open the badge itself: the detail
// is App's slot and the link's answer reaches it directly.

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
}));
vi.mock('../bootPreload', () => ({ takePreloadedSettings: () => null }));

import { useAppStore } from './AppStore';

const get = () => useAppStore.getState();

beforeEach(() => {
  useAppStore.setState({
    showBadgesOverlay: false,
    badgesOverlayInitialPaintId: null,
    badgesOverlayInitialBadgeId: null,
    badgesOverlayInitialStreamNook: false,
    badgesOverlayInitialTarget: null,
  });
});

describe('openTwitchBadge', () => {
  it('opens Badges on the Twitch tab', () => {
    get().openTwitchBadge('clover', '2');
    expect(get().showBadgesOverlay).toBe(true);
    expect(get().badgesOverlayInitialTarget).toEqual({ tab: 'twitch-badges' });
  });

  it('sets no pending badge: the store holds no badge target and no way to take one', () => {
    // Fails against the old store, which held the badge in a field and handed it out once.
    get().openTwitchBadge('clover', '2');
    expect(Object.keys(get())).not.toContain('badgesOverlayInitial' + 'TwitchBadge');
    expect(Object.keys(get())).not.toContain('consumeInitial' + 'TwitchBadge');
  });

  it('clears the other initial targets', () => {
    useAppStore.setState({
      badgesOverlayInitialPaintId: 'p',
      badgesOverlayInitialBadgeId: 'b',
      badgesOverlayInitialStreamNook: 'slug',
    });
    get().openTwitchBadge('clover', '1');
    expect(get().badgesOverlayInitialPaintId).toBeNull();
    expect(get().badgesOverlayInitialBadgeId).toBeNull();
    expect(get().badgesOverlayInitialStreamNook).toBe(false);
  });
});

describe('every other writer of the overlay targets', () => {
  const writers: Array<[string, () => void]> = [
    ['openBadgesWithPaint', () => get().openBadgesWithPaint('p')],
    ['openBadgesWithBadge', () => get().openBadgesWithBadge('b')],
    ['openBadgesOnStreamNook', () => get().openBadgesOnStreamNook('slug')],
    ['openBadgesWithTarget', () => get().openBadgesWithTarget({ tab: 'bttv' })],
  ];

  it.each(writers)('%s leaves the Twitch tab target behind', (_name, write) => {
    get().openTwitchBadge('clover', '1');
    write();
    expect(get().showBadgesOverlay).toBe(true);
    expect(get().badgesOverlayInitialTarget).not.toEqual({ tab: 'twitch-badges' });
  });

  it('setShowBadgesOverlay(false) closes the overlay and drops its targets', () => {
    get().openTwitchBadge('clover', '1');
    get().setShowBadgesOverlay(false);
    expect(get().showBadgesOverlay).toBe(false);
    expect(get().badgesOverlayInitialTarget).toBeNull();
  });
});
