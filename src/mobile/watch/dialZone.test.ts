import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { DIAL_EDGE_GUARD, dialCanStart, swipeDialsOn } from './dialZone';

describe('the brightness and volume swipes switch (SN-269)', () => {
  it('is on until it is switched off', () => {
    expect(swipeDialsOn(undefined)).toBe(true);
    expect(swipeDialsOn({})).toBe(true);
    expect(swipeDialsOn({ swipe_dials: true })).toBe(true);
    expect(swipeDialsOn({ swipe_dials: false })).toBe(false);
  });
  it('off, a landscape press takes the no-dial path (a tap or a chat swipe)', () => {
    const watch = readFileSync(new URL('../screens/WatchScreen.tsx', import.meta.url), 'utf8');
    expect(watch).toMatch(/const dialsOff = !swipeDialsOn\(/);
    expect(watch).toMatch(/if \(offlineRoom \|\| dialsOff \|\| !swipeOk\)/);
  });
});

describe('where a landscape dial can start', () => {
  it('starts over the middle of the picture', () => {
    expect(dialCanStart(200, 400, 0, 0)).toBe(true);
  });
  it('never starts where the notification shade is pulled', () => {
    expect(dialCanStart(10, 400, 0, 0)).toBe(false);
    expect(dialCanStart(DIAL_EDGE_GUARD - 1, 400, 0, 0)).toBe(false);
  });
  it('never starts where the bars or the home swipe come up', () => {
    expect(dialCanStart(395, 400, 0, 0)).toBe(false);
  });
  it('starts right at the edge of the strip', () => {
    expect(dialCanStart(DIAL_EDGE_GUARD, 400, 0, 0)).toBe(true);
    expect(dialCanStart(400 - DIAL_EDGE_GUARD, 400, 0, 0)).toBe(true);
  });
  it('widens to the system gesture strip when that is larger', () => {
    expect(dialCanStart(60, 400, 72, 0)).toBe(false);
    expect(dialCanStart(340, 400, 0, 72)).toBe(false);
  });
});
