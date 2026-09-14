import { describe, expect, it } from 'vitest';
import { formatAgo, formatVodTime, vodProgressFraction, vodProgressLabel } from './vodProgress';

describe('vodProgressFraction', () => {
  it('is null when never watched', () => {
    expect(vodProgressFraction(undefined, 3600)).toBeNull();
  });

  it('uses the stored duration first, the card length second', () => {
    expect(vodProgressFraction({ position_secs: 900, duration_secs: 3600, completed: false }, 100)).toBeCloseTo(0.25);
    expect(vodProgressFraction({ position_secs: 900, duration_secs: 0, completed: false }, 1800)).toBeCloseTo(0.5);
  });

  it('is null with no denominator at all', () => {
    expect(vodProgressFraction({ position_secs: 900, duration_secs: 0, completed: false }, undefined)).toBeNull();
  });

  it('fills the bar for a finished video and clamps overshoot', () => {
    expect(vodProgressFraction({ position_secs: 10, duration_secs: 3600, completed: true }, undefined)).toBe(1);
    expect(vodProgressFraction({ position_secs: 5000, duration_secs: 3600, completed: false }, undefined)).toBe(1);
  });

  it('hides a sliver that would read as a rendering glitch', () => {
    expect(vodProgressFraction({ position_secs: 1, duration_secs: 3600, completed: false }, undefined)).toBeNull();
  });
});

describe('formatVodTime', () => {
  it('formats hours only when needed', () => {
    expect(formatVodTime(5025)).toBe('1:23:45');
    expect(formatVodTime(245)).toBe('4:05');
    expect(formatVodTime(0)).toBe('0:00');
  });

  it('never throws on garbage', () => {
    expect(formatVodTime(Number.NaN)).toBe('0:00');
    expect(formatVodTime(-5)).toBe('0:00');
  });
});

describe('vodProgressLabel', () => {
  it('mirrors the Rust resume floor', () => {
    expect(vodProgressLabel({ position_secs: 12, duration_secs: 3600, completed: false })).toBeNull();
    expect(vodProgressLabel({ position_secs: 5025, duration_secs: 3600, completed: false })).toBe('Resume at 1:23:45');
    expect(vodProgressLabel({ position_secs: 3590, duration_secs: 3600, completed: true })).toBe('Watched');
    expect(vodProgressLabel(undefined)).toBeNull();
  });
});

describe('formatAgo', () => {
  it('rounds to minutes and hours', () => {
    expect(formatAgo(20)).toBe('just now');
    expect(formatAgo(42 * 60 + 10)).toBe('42m ago');
    expect(formatAgo(3600 + 5 * 60)).toBe('1h 05m ago');
  });
});
