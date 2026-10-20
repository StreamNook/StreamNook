import { describe, expect, it } from 'vitest';
import { GRAB_ZONE, inGrabZone, messageReach, slideStart, stepAction, stepAt, stepKeeping, stepTint, type TouchSlider } from './swipeSteps';

// Rust's own test ruler (touch_mod_slider.rs): Delete, 1m, 10m, 1h, a gap, Ban on a 390 px row.
const slider: TouchSlider = {
  enabled: true,
  style: 'preview',
  steps: [
    { action: 'delete', seconds: null, label: 'Delete', chip: 'Delete message', tone: null },
    { action: 'timeout', seconds: 60, label: '1m', chip: 'Time out 1m', tone: 0 },
    { action: 'timeout', seconds: 600, label: '10m', chip: 'Time out 10m', tone: 0.325 },
    { action: 'timeout', seconds: 3600, label: '1h', chip: 'Time out 1h', tone: 0.65 },
    { action: 'ban', seconds: null, label: 'Ban', chip: 'Ban', tone: 1 },
  ],
  dead: 72,
  spacing: 34,
  starts: [72, 106, 140, 174, 242],
  end: 293,
  ban_dwell_ms: 350,
  ban_still: 10,
  hysteresis: 4,
  leash: 120,
  engage: 10,
  cancel_words: 'Let go to cancel',
  hold_ban_words: 'Hold for Ban, let go to cancel',
  nudge_words: 'Swipe right on a message to moderate it',
  start: 0,
  quick_seconds: 60,
};

describe('swipe slider steps (same pulls as Rust)', () => {
  it('reads the cancel zone, the gap before Ban and Ban', () => {
    expect(stepAt(slider, 10)).toBeNull();
    expect(stepAt(slider, 76)).toBe(0);
    expect(stepAt(slider, 176)).toBe(3);
    expect(stepAt(slider, 216)).toBe(3);
    expect(stepAt(slider, 246)).toBe(4);
    expect(stepAt({ ...slider, steps: [slider.steps[4]], starts: [106] }, 76)).toBeNull();
  });

  it('holds a step near its edge', () => {
    expect(stepKeeping(slider, 107, 0)).toBe(0);
    expect(stepKeeping(slider, 111, 0)).toBe(1);
    expect(stepKeeping(slider, 104, 1)).toBe(1);
    expect(stepKeeping(slider, 101, 1)).toBe(0);
    expect(stepKeeping(slider, 20, 1)).toBeNull();
  });

  it('names each step as the mod door takes it, in its color', () => {
    expect(stepAction(slider.steps[0], 'm1')).toEqual({ kind: 'delete', messageId: 'm1' });
    expect(stepAction(slider.steps[2], 'm1')).toEqual({ kind: 'timeout', seconds: 600 });
    expect(stepAction(slider.steps[4], 'm1')).toEqual({ kind: 'ban' });
    expect(stepTint(slider.steps[4])).toBe('var(--color-error)');
    expect(stepTint(slider.steps[3])).toContain('65%');
  });
});

describe('what a message allows (Twitch rules)', () => {
  it('lets a moderator delete their own message and nothing more', () => {
    expect(messageReach('moderator', '7', '7', ['moderator/1'])).toBe('delete_only');
  });
  it('never acts on the broadcaster, not even in their own room', () => {
    expect(messageReach('broadcaster', '1', '1', [])).toBe('nothing');
    expect(messageReach('moderator', '7', '1', ['broadcaster/1'])).toBe('nothing');
  });
  it('keeps moderators apart, except for the broadcaster', () => {
    expect(messageReach('moderator', '7', '8', ['moderator/1'])).toBe('nothing');
    expect(messageReach('broadcaster', '1', '8', ['moderator/1'])).toBe('no_delete');
    expect(messageReach('moderator', '7', '9', ['subscriber/12'])).toBe('all');
    expect(messageReach(null, '7', '9', [])).toBe('nothing');
  });
});

describe('where a slide can start', () => {
  it('has a grab zone a thumb wide, just past the system edge strip', () => {
    // A 24 pt back strip: the zone runs from 24 to 80.
    expect(inGrabZone(23, 24)).toBe(false);
    expect(inGrabZone(24, 24)).toBe(true);
    expect(inGrabZone(24 + GRAB_ZONE - 1, 24)).toBe(true);
    expect(inGrabZone(24 + GRAB_ZONE, 24)).toBe(false);
    // No strip (three-button navigation): it starts at the screen edge.
    expect(inGrabZone(0, 0)).toBe(true);
  });

  it('keeps the strict rule outside the zone, so scrolling stays the same', () => {
    expect(slideStart(12, 3, 10, false)).toBe('slide');
    expect(slideStart(8, 2, 10, false)).toBe('wait');
    expect(slideStart(12, 12, 10, false)).toBe('scroll');
    expect(slideStart(9, 16, 10, false)).toBe('scroll');
  });

  it('starts a thumb arc from the edge in the grab zone', () => {
    // 7 right and 2 down: too short outside, a slide in the zone.
    expect(slideStart(7, 2, 10, true)).toBe('slide');
    // A 60 degree arc: a scroll outside, a slide in the zone.
    expect(slideStart(9, 16, 10, false)).toBe('scroll');
    expect(slideStart(9, 16, 10, true)).toBe('slide');
    // Mostly vertical is still the list's scroll there too.
    expect(slideStart(4, 20, 10, true)).toBe('scroll');
    // Leftward never slides.
    expect(slideStart(-8, 1, 10, true)).toBe('wait');
  });
});
