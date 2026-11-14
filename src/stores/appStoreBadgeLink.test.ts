import { beforeEach, describe, expect, it, vi } from 'vitest';

// The badge-link target in the store: openTwitchBadge opens the Twitch tab with
// it, every other writer of the overlay's initial fields clears it, and
// consuming returns it once.

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
    badgesOverlayInitialTwitchBadge: null,
  });
});

describe('openTwitchBadge', () => {
  it('opens Badges on the Twitch tab with the badge pending', () => {
    get().openTwitchBadge('clover', '2');
    expect(get().showBadgesOverlay).toBe(true);
    expect(get().badgesOverlayInitialTarget).toEqual({ tab: 'twitch-badges' });
    expect(get().badgesOverlayInitialTwitchBadge).toEqual({ setId: 'clover', version: '2' });
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

describe('every other writer clears the pending badge', () => {
  const writers: Array<[string, () => void]> = [
    ['openBadgesWithPaint', () => get().openBadgesWithPaint('p')],
    ['openBadgesWithBadge', () => get().openBadgesWithBadge('b')],
    ['openBadgesOnStreamNook', () => get().openBadgesOnStreamNook('slug')],
    ['openBadgesWithTarget', () => get().openBadgesWithTarget({ tab: 'bttv' })],
    ['setShowBadgesOverlay(false)', () => get().setShowBadgesOverlay(false)],
  ];
  it.each(writers)('%s', (_name, write) => {
    get().openTwitchBadge('clover', '1');
    write();
    expect(get().badgesOverlayInitialTwitchBadge).toBeNull();
  });
});

describe('consumeInitialTwitchBadge', () => {
  it('returns the target once, then null', () => {
    get().openTwitchBadge('clover', '3');
    expect(get().consumeInitialTwitchBadge()).toEqual({ setId: 'clover', version: '3' });
    expect(get().badgesOverlayInitialTwitchBadge).toBeNull();
    expect(get().consumeInitialTwitchBadge()).toBeNull();
  });

  it('keeps the overlay open and on the Twitch tab', () => {
    get().openTwitchBadge('clover', '1');
    get().consumeInitialTwitchBadge();
    expect(get().showBadgesOverlay).toBe(true);
    expect(get().badgesOverlayInitialTarget).toEqual({ tab: 'twitch-badges' });
  });
});
