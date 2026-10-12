import { describe, expect, it } from 'vitest';
import {
  PAGES_OVER_PLAYER,
  TITLE_BAR_REVEAL_ZONE_PX,
  aspectTitleBarReserve,
  barClickExitsPlayerFullscreen,
  opensPageOverPlayer,
  pageOverPlayerOpen,
  pointerAtTopEdge,
  shouldDoubleClickMaximize,
  shouldStartWindowDrag,
  titleBarAutoHides,
  titleBarLifted,
  titleBarTuckMode,
  titleBarTucked,
  watchingForTitleBar,
} from './titleBarTuck';
import { LINUX_RESIZE_EDGE_PX, WINDOWS_TOP_RESIZE_BORDER_PX } from './windowResizeEdges';

const base = { hideWhileWatching: false, watching: false, windowFullscreen: false, streamOnlyFullscreen: false };

describe('titleBarAutoHides', () => {
  it('hides only while watching with the setting on', () => {
    expect(titleBarAutoHides({ ...base, hideWhileWatching: true, watching: true })).toBe(true);
    expect(titleBarAutoHides({ ...base, hideWhileWatching: true, watching: false })).toBe(false);
    expect(titleBarAutoHides({ ...base, hideWhileWatching: false, watching: true })).toBe(false);
  });

  it('leaves full screen to its own rule', () => {
    expect(titleBarAutoHides({ ...base, windowFullscreen: true, streamOnlyFullscreen: true })).toBe(true);
    // The setting does not tuck a full screen that keeps its chrome.
    expect(
      titleBarAutoHides({ hideWhileWatching: true, watching: true, windowFullscreen: true, streamOnlyFullscreen: false }),
    ).toBe(false);
  });
});

describe('titleBarTuckMode', () => {
  it("tucks in the player's own full screen even with the layout mode off", () => {
    expect(titleBarTuckMode({ layoutAutoHides: false, playerFullscreen: true })).toBe(true);
    expect(titleBarTuckMode({ layoutAutoHides: true, playerFullscreen: false })).toBe(true);
    expect(titleBarTuckMode({ layoutAutoHides: false, playerFullscreen: false })).toBe(false);
  });
});

describe('titleBarTucked', () => {
  it('tucks in the mode unless revealed or in use', () => {
    expect(titleBarTucked({ autoHides: true, revealed: false, inUse: false })).toBe(true);
    expect(titleBarTucked({ autoHides: true, revealed: true, inUse: false })).toBe(false);
    expect(titleBarTucked({ autoHides: true, revealed: false, inUse: true })).toBe(false);
  });

  it('never tucks outside the mode', () => {
    expect(titleBarTucked({ autoHides: false, revealed: false, inUse: false })).toBe(false);
  });
});

describe('aspectTitleBarReserve', () => {
  it('counts the row only while it is reserved', () => {
    expect(aspectTitleBarReserve(false)).toBe(40);
    expect(aspectTitleBarReserve(true)).toBe(0);
  });
});

describe('shouldDoubleClickMaximize', () => {
  it('maximizes outside full screen only', () => {
    expect(shouldDoubleClickMaximize({ fullscreen: false })).toBe(true);
    expect(shouldDoubleClickMaximize({ fullscreen: true })).toBe(false);
  });
});

describe('barClickExitsPlayerFullscreen', () => {
  const nav = { playerFullscreen: true, actionable: true, opensMenu: false, keepsFullscreen: false };

  it('leaves the player full screen before a bar control changes the page', () => {
    expect(barClickExitsPlayerFullscreen(nav)).toBe(true);
  });

  it('keeps full screen for menus, window buttons and empty bar', () => {
    expect(barClickExitsPlayerFullscreen({ ...nav, opensMenu: true })).toBe(false);
    expect(barClickExitsPlayerFullscreen({ ...nav, keepsFullscreen: true })).toBe(false);
    expect(barClickExitsPlayerFullscreen({ ...nav, actionable: false })).toBe(false);
  });

  it('does nothing outside the player full screen', () => {
    expect(barClickExitsPlayerFullscreen({ ...nav, playerFullscreen: false })).toBe(false);
  });
});

describe('watchingForTitleBar', () => {
  const at = { isHomeActive: false, streamUrl: 'https://relay/stream.m3u8', isMultiNookActive: false };

  it('a stream, VOD or MultiNook grid in front counts', () => {
    expect(watchingForTitleBar(at)).toBe(true);
    expect(watchingForTitleBar({ ...at, streamUrl: null, isMultiNookActive: true })).toBe(true);
  });

  it('Home, nothing playing and the offline room do not', () => {
    expect(watchingForTitleBar({ ...at, isHomeActive: true })).toBe(false);
    expect(watchingForTitleBar({ ...at, streamUrl: null })).toBe(false);
    expect(watchingForTitleBar({ ...at, streamUrl: 'offline' })).toBe(false);
  });
});

describe('top-edge reveal zone', () => {
  it('reaches past the pixels the platforms keep for window resizing', () => {
    expect(TITLE_BAR_REVEAL_ZONE_PX).toBeGreaterThan(LINUX_RESIZE_EDGE_PX);
    expect(TITLE_BAR_REVEAL_ZONE_PX).toBeGreaterThan(WINDOWS_TOP_RESIZE_BORDER_PX);
  });

  it('counts the zone, its last pixel and a pointer that left through the top', () => {
    expect(pointerAtTopEdge(0)).toBe(true);
    expect(pointerAtTopEdge(TITLE_BAR_REVEAL_ZONE_PX)).toBe(true);
    expect(pointerAtTopEdge(-4)).toBe(true);
    expect(pointerAtTopEdge(TITLE_BAR_REVEAL_ZONE_PX + 1)).toBe(false);
    expect(pointerAtTopEdge(300)).toBe(false);
  });
});

describe('opensPageOverPlayer', () => {
  const closed = Object.fromEntries(PAGES_OVER_PLAYER.map((k) => [k, k === 'isHomeActive' ? false : null]));

  it('any page, overlay or modal opening counts', () => {
    for (const key of PAGES_OVER_PLAYER) {
      expect(opensPageOverPlayer(closed, { ...closed, [key]: key.startsWith('profile') ? { id: '1' } : true })).toBe(true);
    }
  });

  it('one already open, closing, or nothing changing does not', () => {
    const open = { ...closed, isSettingsOpen: true };
    expect(opensPageOverPlayer(open, open)).toBe(false);
    expect(opensPageOverPlayer(open, closed)).toBe(false);
    expect(opensPageOverPlayer(closed, closed)).toBe(false);
  });

  it('flags outside the list are ignored', () => {
    expect(opensPageOverPlayer(closed, { ...closed, isTheaterMode: true } as typeof closed)).toBe(false);
  });
});

describe('shouldStartWindowDrag', () => {
  it('drags outside full screen only', () => {
    expect(shouldStartWindowDrag({ fullscreen: false })).toBe(true);
    expect(shouldStartWindowDrag({ fullscreen: true })).toBe(false);
  });
});

// The player's top-right buttons and title overlays share the bar's resting
// layer and come later in the page, so a bar coming back over the player (the
// tucking layout, the player's full screen) has to lift above them or its
// controls are drawn and clicked under the player's.
describe('titleBarLifted', () => {
  const at = { tuckMode: false, playerFullscreen: false, pageOpen: false };

  it('lifts for as long as the bar tucks over the player', () => {
    expect(titleBarLifted({ ...at, tuckMode: true })).toBe(true);
  });

  it('stays at its resting layer while it reserves its row', () => {
    expect(titleBarLifted(at)).toBe(false);
  });

  it('a page open over the stream keeps it under that page', () => {
    expect(titleBarLifted({ ...at, tuckMode: true, pageOpen: true })).toBe(false);
  });

  it("always lifts over the player's full screen", () => {
    expect(titleBarLifted({ ...at, tuckMode: true, playerFullscreen: true })).toBe(true);
    expect(titleBarLifted({ ...at, tuckMode: true, playerFullscreen: true, pageOpen: true })).toBe(true);
  });
});

describe('pageOverPlayerOpen', () => {
  const closed = Object.fromEntries(PAGES_OVER_PLAYER.map((k) => [k, k === 'isHomeActive' ? false : null]));

  it('any page, overlay or modal in the list counts while open', () => {
    expect(pageOverPlayerOpen(closed)).toBe(false);
    for (const key of PAGES_OVER_PLAYER) {
      expect(pageOverPlayerOpen({ ...closed, [key]: key.startsWith('profile') ? { id: '1' } : true })).toBe(true);
    }
  });
});
