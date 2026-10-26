import { describe, expect, it } from 'vitest';
import { holdStill } from './holdStill';

describe('holdStill', () => {
  it('skips a render only while the hold stays on', () => {
    expect(holdStill({ hold: true }, { hold: true })).toBe(true);
    expect(holdStill({ hold: false }, { hold: true })).toBe(false);
    expect(holdStill({ hold: true }, { hold: false })).toBe(false);
    expect(holdStill({ hold: false }, { hold: false })).toBe(false);
  });
});
