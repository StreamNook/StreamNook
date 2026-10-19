import { describe, expect, it } from 'vitest';
import { stepAction, stepAt, stepKeeping, stepTint, type TouchSlider } from './swipeSteps';

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
