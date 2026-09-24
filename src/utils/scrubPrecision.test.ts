import { describe, it, expect } from 'vitest';
import { broadcastScale } from './broadcastScale';
import { dragDivisor, dragSpeedLabel, FINE_PX, FINEST_PX, nudgeFrac, wheelStepSecs } from './scrubPrecision';

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

  it('a mostly horizontal wheel goes right = forward', () => {
    expect(wheelStepSecs(ev({ deltaX: 50, deltaY: 10 }))).toBe(10);
    expect(wheelStepSecs(ev({ deltaX: -50 }))).toBe(-10);
  });

  it('a null event nudges nothing', () => {
    expect(wheelStepSecs(ev({}))).toBe(0);
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
