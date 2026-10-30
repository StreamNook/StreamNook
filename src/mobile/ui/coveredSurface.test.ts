import { describe, expect, it } from 'vitest';
import {
  closesCoveredMenu,
  focusUnder,
  inboxPaneOnScreen,
  menuOutOfSight,
  OUT_OF_SIGHT_SELECTOR,
  refuseFocusIn,
  watchLayerUp,
} from './coveredSurface';

/** A surface that is also the focus target, counting blurs. */
class FakeSurface extends EventTarget {
  blurred = 0;
  blur() {
    this.blurred += 1;
  }
}

describe('refuseFocusIn', () => {
  it('blurs whatever takes focus inside the surface while it is out of sight', () => {
    const surface = new FakeSurface();
    const undo = refuseFocusIn(surface);
    surface.dispatchEvent(new Event('focusin'));
    expect(surface.blurred).toBe(1);
    undo();
  });

  it('lets focus be once the surface is back in sight', () => {
    const surface = new FakeSurface();
    const undo = refuseFocusIn(surface);
    undo();
    surface.dispatchEvent(new Event('focusin'));
    expect(surface.blurred).toBe(0);
  });
});

describe('watchLayerUp', () => {
  it('full with a stream, a load or an offline room is up', () => {
    expect(watchLayerUp('full', 'https://x/m.m3u8', false)).toBe(true);
    expect(watchLayerUp('full', null, true)).toBe(true);
    expect(watchLayerUp('full', 'offline', false)).toBe(true);
  });

  it('the mini box or nothing playing is not', () => {
    expect(watchLayerUp('mini', 'https://x/m.m3u8', false)).toBe(false);
    expect(watchLayerUp('full', null, false)).toBe(false);
  });
});

describe('inboxPaneOnScreen', () => {
  it('the player chat while the layer is up, Chats while it is not', () => {
    expect(inboxPaneOnScreen('watch', true, false, false)).toBe(true);
    expect(inboxPaneOnScreen('chats', true, false, false)).toBe(false);
    expect(inboxPaneOnScreen('watch', false, false, false)).toBe(false);
    expect(inboxPaneOnScreen('chats', false, false, false)).toBe(true);
  });

  it('a pane idle or paused is never on screen', () => {
    expect(inboxPaneOnScreen('watch', true, true, false)).toBe(false);
    expect(inboxPaneOnScreen('chats', false, false, true)).toBe(false);
  });
});

/** A stand-in element: inside an out-of-sight container or not. */
const part = (hidden: boolean) => ({
  closest: (selector: string) => (hidden && selector === OUT_OF_SIGHT_SELECTOR ? {} : null),
});

describe('menuOutOfSight', () => {
  it('every part hidden: out of sight', () => {
    expect(menuOutOfSight([part(true), part(true)])).toBe(true);
  });

  it('one part in sight (portaled out): in sight', () => {
    expect(menuOutOfSight([part(true), part(false)])).toBe(false);
  });

  it('missing parts are skipped; none at all is in sight', () => {
    expect(menuOutOfSight([null, part(true), undefined])).toBe(true);
    expect(menuOutOfSight([null, undefined])).toBe(false);
    expect(menuOutOfSight([])).toBe(false);
  });
});

describe('focusUnder', () => {
  const field = { id: 'chats-composer' };
  const outside = { id: 'player-composer' };
  const shell = { contains: (node: { id: string }) => node === field };

  it('returns the focused field inside the surface going out of sight', () => {
    expect(focusUnder(shell, field)).toBe(field);
  });

  it('leaves focus outside that surface alone', () => {
    expect(focusUnder(shell, outside)).toBeNull();
  });

  it('does nothing with no surface or no focus', () => {
    expect(focusUnder(null, field)).toBeNull();
    expect(focusUnder(shell, null)).toBeNull();
  });
});

describe('closesCoveredMenu', () => {
  it('a menu in sight closes on any outside press', () => {
    expect(closesCoveredMenu(false, false)).toBe(true);
    expect(closesCoveredMenu(false, true)).toBe(true);
  });

  it('a covered menu ignores a press on the layer above it', () => {
    expect(closesCoveredMenu(true, false)).toBe(false);
  });

  it('a covered menu still closes on a press on its own covered surface', () => {
    expect(closesCoveredMenu(true, true)).toBe(true);
  });
});
