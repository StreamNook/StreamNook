import { describe, expect, it } from 'vitest';
import { parseTimeoutSeconds } from './timeoutDuration';

describe('parseTimeoutSeconds', () => {
  it('reads plain seconds', () => {
    expect(parseTimeoutSeconds('600')).toBe(600);
  });

  it("reads Twitch's unit forms", () => {
    expect(parseTimeoutSeconds('10m')).toBe(600);
    expect(parseTimeoutSeconds('30s')).toBe(30);
    expect(parseTimeoutSeconds('1h30m')).toBe(5400);
    expect(parseTimeoutSeconds('2d')).toBe(172800);
    expect(parseTimeoutSeconds('1W')).toBe(604800);
  });

  it('refuses anything that is not a duration', () => {
    expect(parseTimeoutSeconds('spamming')).toBeNull();
    expect(parseTimeoutSeconds('10x')).toBeNull();
    expect(parseTimeoutSeconds('0')).toBeNull();
    expect(parseTimeoutSeconds('')).toBeNull();
  });
});
