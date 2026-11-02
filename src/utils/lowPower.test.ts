import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  isBatteryLowPower,
  isFlat,
  isLowPower,
  isPerformanceQuiet,
  isStillChat,
  setLowPower,
  setLowPowerSource,
  subscribeLowPower,
} from './lowPower';

const OFF = { quiet: false, still: false, flat: false };

afterEach(() => {
  setLowPower(false);
  setLowPowerSource('performance', OFF);
  vi.unstubAllGlobals();
});

describe('lowPower', () => {
  it('notifies once per real change', () => {
    let calls = 0;
    const off = subscribeLowPower(() => {
      calls += 1;
    });
    expect(setLowPower(false)).toBe(false);
    expect(calls).toBe(0);
    expect(setLowPower(true)).toBe(true);
    expect(setLowPower(true)).toBe(false);
    expect(isLowPower()).toBe(true);
    expect(calls).toBe(1);
    expect(setLowPower(false)).toBe(true);
    expect(calls).toBe(2);
    off();
    setLowPower(true);
    expect(calls).toBe(2);
    setLowPower(false);
  });

  it('Battery Saver is quiet and still, never flat', () => {
    setLowPower(true);
    expect([isLowPower(), isBatteryLowPower(), isStillChat(), isFlat(), isPerformanceQuiet()]).toEqual([
      true,
      true,
      true,
      false,
      false,
    ]);
  });

  it('the two sources compose: one turning off never clears the other', () => {
    setLowPowerSource('performance', { quiet: true, still: false, flat: true });
    setLowPower(true);
    setLowPower(false);
    expect(isLowPower()).toBe(true);
    expect(isPerformanceQuiet()).toBe(true);
    expect(isBatteryLowPower()).toBe(false);
    expect(isStillChat()).toBe(false);
    expect(isFlat()).toBe(true);
    setLowPower(true);
    setLowPowerSource('performance', OFF);
    expect(isLowPower()).toBe(true);
    expect(isStillChat()).toBe(true);
    expect(isFlat()).toBe(false);
  });

  it('still follows its own field, not quiet', () => {
    setLowPowerSource('performance', { quiet: true, still: false, flat: true });
    expect(isStillChat()).toBe(false);
    setLowPowerSource('performance', { quiet: true, still: true, flat: true });
    expect(isStillChat()).toBe(true);
  });

  it('notifies only when a derived value changes', () => {
    let calls = 0;
    const off = subscribeLowPower(() => {
      calls += 1;
    });
    setLowPowerSource('performance', { quiet: true, still: false, flat: true });
    expect(calls).toBe(1);
    // Battery quiet is new; still is new: one change, one call.
    setLowPower(true);
    expect(calls).toBe(2);
    // Same look again: nothing.
    expect(setLowPowerSource('performance', { quiet: true, still: false, flat: true })).toBe(false);
    expect(calls).toBe(2);
    off();
  });

  it('writes the page attributes from the derived values', () => {
    const dataset: Record<string, string> = {};
    vi.stubGlobal('document', { documentElement: { dataset } });
    setLowPowerSource('performance', { quiet: true, still: true, flat: true });
    expect(dataset).toEqual({ snLowPower: 'true', snPerf: 'true', snStillChat: 'true' });
    setLowPowerSource('performance', { quiet: true, still: false, flat: true });
    expect(dataset).toEqual({ snLowPower: 'true', snPerf: 'true' });
    setLowPower(true);
    setLowPowerSource('performance', OFF);
    // Battery quiet keeps the loop cap but never the instant look.
    expect(dataset).toEqual({ snLowPower: 'true', snStillChat: 'true' });
    setLowPower(false);
    expect(dataset).toEqual({});
  });
});
