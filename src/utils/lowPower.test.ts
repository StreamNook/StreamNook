import { describe, expect, it } from 'vitest';
import { isLowPower, setLowPower, subscribeLowPower } from './lowPower';

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
});
