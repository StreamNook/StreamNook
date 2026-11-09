import { describe, expect, it } from 'vitest';
import { ownBadgeHidden } from './chatterBadges';

describe('ownBadgeHidden', () => {
  it('keeps your badge while the switch is on or unset', () => {
    expect(ownBadgeHidden('1', undefined, '1', [])).toBe(false);
    expect(ownBadgeHidden('1', true, '1', [])).toBe(false);
  });

  it('takes it off your own accounts only while the switch is off', () => {
    expect(ownBadgeHidden('1', false, '1', [])).toBe(true);
    expect(ownBadgeHidden('2', false, '1', ['2'])).toBe(true);
    expect(ownBadgeHidden('3', false, '1', ['2'])).toBe(false);
    expect(ownBadgeHidden(null, false, '1', [])).toBe(false);
    expect(ownBadgeHidden('1', false, null, [])).toBe(false);
  });
});
