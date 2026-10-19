import { describe, expect, it } from 'vitest';
import { customMinutes, sleepTimerRunning } from './sleepTimerModel';

describe('sleep timer helpers', () => {
  it('a custom pick comes to minutes, capped at a day', () => {
    expect(customMinutes(1, 30)).toBe(90);
    expect(customMinutes(0, 1)).toBe(1);
    expect(customMinutes(0, 0)).toBeNull();
    expect(customMinutes(30, 0)).toBe(1440);
  });

  it('only running and fading count as running', () => {
    const base = { fadeSecs: 10, presets: [] };
    expect(sleepTimerRunning({ ...base, phase: 'running' })).toBe(true);
    expect(sleepTimerRunning({ ...base, phase: 'fading' })).toBe(true);
    expect(sleepTimerRunning({ ...base, phase: 'fired' })).toBe(false);
    expect(sleepTimerRunning({ ...base, phase: 'off' })).toBe(false);
    expect(sleepTimerRunning(null)).toBe(false);
  });
});
