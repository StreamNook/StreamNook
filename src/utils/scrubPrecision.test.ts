import { describe, it, expect } from 'vitest';
import { broadcastScale } from './broadcastScale';
import { dragDivisor, dragFrac, dragSpeedLabel, FINE_PX, FINEST_PX, nudgeFrac, wheelStepSecs } from './scrubPrecision';

describe('dragDivisor', () => {
  it('is full speed on the bar and slows in two steps away from it', () => {
    expect(dragDivisor(0)).toBe(1);
    expect(dragDivisor(FINE_PX - 1)).toBe(1);
    expect(dragDivisor(FINE_PX)).toBe(4);
    expect(dragDivisor(FINEST_PX - 1)).toBe(4);
    expect(dragDivisor(FINEST_PX)).toBe(16);
    expect(dragDivisor(500)).toBe(16);
  });

  it('treats a nonsense distance as on the bar', () => {
    expect(dragDivisor(Number.NaN)).toBe(1);
    expect(dragDivisor(-10)).toBe(1);
  });

  it('labels only the slowed speeds', () => {
    expect(dragSpeedLabel(1)).toBe('');
    expect(dragSpeedLabel(4)).toBe('1/4 speed');
    expect(dragSpeedLabel(16)).toBe('1/16 speed');
  });
});

describe('wheelStepSecs', () => {
  const ev = (o: Partial<{ deltaX: number; deltaY: number; shiftKey: boolean; ctrlKey: boolean }>) => ({
    deltaX: 0,
    deltaY: 0,
    shiftKey: false,
    ctrlKey: false,
    ...o,
  });

  it('wheel up goes forward ten seconds, wheel down back, whatever the delta size', () => {
    expect(wheelStepSecs(ev({ deltaY: -100 }))).toBe(10);
    expect(wheelStepSecs(ev({ deltaY: -3 }))).toBe(10);
    expect(wheelStepSecs(ev({ deltaY: 120 }))).toBe(-10);
  });

  it('shift is a minute, ctrl a second', () => {
    expect(wheelStepSecs(ev({ deltaY: -1, shiftKey: true }))).toBe(60);
    expect(wheelStepSecs(ev({ deltaY: 1, ctrlKey: true }))).toBe(-1);
  });

  it('shift + wheel, which Chromium delivers as a horizontal scroll, keeps the wheel direction', () => {
    // Wheel up with Shift arrives as deltaX < 0 and deltaY 0.
    expect(wheelStepSecs(ev({ deltaX: -100, shiftKey: true }))).toBe(60);
    expect(wheelStepSecs(ev({ deltaX: 100, shiftKey: true }))).toBe(-60);
  });

  it('a mostly horizontal wheel goes right = forward', () => {
    expect(wheelStepSecs(ev({ deltaX: 50, deltaY: 10 }))).toBe(10);
    expect(wheelStepSecs(ev({ deltaX: -50 }))).toBe(-10);
  });

  it('a null event nudges nothing', () => {
    expect(wheelStepSecs(ev({}))).toBe(0);
  });
});

describe('dragFrac', () => {
  it('moves a pixel per pixel at full speed and a sixteenth when slowed', () => {
    expect(dragFrac(0.5, 8, 800, 1)).toBeCloseTo(0.51, 9);
    expect(dragFrac(0.5, 16, 800, 16)).toBeCloseTo(0.50125, 9);
    expect(dragFrac(0.5, -8, 800, 4)).toBeCloseTo(0.4975, 9);
  });

  it('clamps to the bar and ignores a bar with no width', () => {
    expect(dragFrac(0.99, 100, 800, 1)).toBe(1);
    expect(dragFrac(0.01, -100, 800, 1)).toBe(0);
    expect(dragFrac(0.3, 50, 0, 1)).toBe(0.3);
  });
});

describe('nudgeFrac', () => {
  const total = 4 * 3600;
  const scale = broadcastScale(total);

  it('moves the same number of seconds in the coarse and the fine section', () => {
    for (const frac of [0.1, 0.95]) {
      const before = scale.toSecs(frac);
      const after = scale.toSecs(nudgeFrac(scale, total, frac, 10));
      expect(after - before).toBeCloseTo(10, 3);
    }
  });

  it('clamps to the broadcast at both ends', () => {
    expect(nudgeFrac(scale, total, 0, -60)).toBe(0);
    expect(nudgeFrac(scale, total, 1, 60)).toBe(1);
  });
});
