import { describe, expect, it } from 'vitest';
import { agoLabel, durationLabel, lastLiveLabel, nextStreamLabel } from './lastLive';

const NOW = new Date(2026, 9, 9, 12, 0, 0).getTime();
const ago = (ms: number) => new Date(NOW - ms).toISOString();
const ahead = (ms: number) => new Date(NOW + ms).toISOString();
const H = 3_600_000;

describe('agoLabel', () => {
  it('words each range', () => {
    expect(agoLabel(ago(10_000), NOW)).toBe('just now');
    expect(agoLabel(ago(12 * 60_000), NOW)).toBe('12m ago');
    expect(agoLabel(ago(3 * H), NOW)).toBe('3h ago');
    expect(agoLabel(ago(30 * H), NOW)).toBe('yesterday');
    expect(agoLabel(ago(4 * 24 * H), NOW)).toBe('4d ago');
    expect(agoLabel(ago(65 * 24 * H), NOW)).toBe('2mo ago');
    expect(agoLabel(ago(400 * 24 * H), NOW)).toBe('1y ago');
  });

  it('is null for nothing or garbage, and never negative', () => {
    expect(agoLabel(null, NOW)).toBeNull();
    expect(agoLabel('not a date', NOW)).toBeNull();
    expect(agoLabel(ahead(H), NOW)).toBe('just now');
  });
});

describe('lastLiveLabel', () => {
  it('says Offline when the time is unknown', () => {
    expect(lastLiveLabel(undefined, NOW)).toBe('Offline');
    expect(lastLiveLabel(ago(3 * H), NOW)).toBe('Last live 3h ago');
  });
});

describe('nextStreamLabel', () => {
  it('names the day relative to today', () => {
    expect(nextStreamLabel(ago(H), NOW)).toBe('Now');
    expect(nextStreamLabel(ahead(2 * H), NOW)).toMatch(/^Today at /);
    expect(nextStreamLabel(ahead(24 * H), NOW)).toMatch(/^Tomorrow at /);
    expect(nextStreamLabel(ahead(3 * 24 * H), NOW)).toMatch(/^\w{2,4}\.? at /);
    expect(nextStreamLabel(null, NOW)).toBeNull();
  });
});

describe('durationLabel', () => {
  it('reads like a VOD badge', () => {
    expect(durationLabel(45373)).toBe('12h 36m');
    expect(durationLabel(7200)).toBe('2h');
    expect(durationLabel(2700)).toBe('45m');
    expect(durationLabel(20)).toBe('under a minute');
  });
});
