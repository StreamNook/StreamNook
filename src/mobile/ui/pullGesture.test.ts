import { describe, expect, it } from 'vitest';
import { pullDecision } from './pullGesture';

describe('pullDecision', () => {
  it('waits inside the slop, then claims a pull that is more down than sideways', () => {
    expect(pullDecision(3, 9, 0, true)).toBe('wait');
    expect(pullDecision(3, 12, 0, true)).toBe('claim');
  });

  it('releases a sideways swipe, an upward move, a scrolled list and a move the browser already scrolls', () => {
    expect(pullDecision(12, 3, 0, true)).toBe('release');
    expect(pullDecision(0, -12, 0, true)).toBe('release');
    expect(pullDecision(0, 12, 5, true)).toBe('release');
    expect(pullDecision(9, 11, 0, false)).toBe('release');
  });
});
