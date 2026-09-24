import { describe, expect, it } from 'vitest';
import {
  VOD_FALLBACK_THUMB,
  formatAgo,
  formatRemaining,
  formatVodTime,
  parseHelixDuration,
  videoDurationLabel,
  vodProgressFraction,
  vodProgressLabel,
  vodThumbUrl,
} from './vodProgress';

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
    expect(formatAgo(20)).toBe('20s ago');
    expect(formatAgo(42 * 60 + 10)).toBe('42m ago');
    expect(formatAgo(3600 + 5 * 60)).toBe('1h 05m ago');
  });
});

describe('formatRemaining', () => {
  it('reads as hours and minutes left', () => {
    expect(formatRemaining(3 * 3600 + 47 * 60 + 39, 5 * 3600 + 41 * 60 + 27)).toBe('1h 53m left');
    expect(formatRemaining(0, 2 * 3600)).toBe('2h left');
    expect(formatRemaining(600, 1500)).toBe('15m left');
    expect(formatRemaining(3570, 3600)).toBe('Under a minute left');
  });

  it('is null without a duration', () => {
    expect(formatRemaining(100, 0)).toBeNull();
    expect(formatRemaining(100, Number.NaN)).toBeNull();
  });
});

describe('videoDurationLabel', () => {
  it('parses the Helix shape and prefers the exact length', () => {
    expect(parseHelixDuration('7h19m49s')).toBe(7 * 3600 + 19 * 60 + 49);
    expect(parseHelixDuration('9m43s')).toBe(9 * 60 + 43);
    expect(parseHelixDuration('42s')).toBe(42);
    expect(parseHelixDuration('')).toBeNull();
    expect(parseHelixDuration('1:02:03')).toBeNull();
    expect(videoDurationLabel({ duration: '7h19m49s' })).toBe('7:19:49');
    expect(videoDurationLabel({ duration: '7h19m49s', length_seconds: 26389 })).toBe('7:19:49');
    expect(videoDurationLabel({ duration: '9m43s' })).toBe('9:43');
    expect(videoDurationLabel({ duration: 'weird' })).toBe('weird');
  });
});

describe('vodThumbUrl', () => {
  it('substitutes the width and height placeholders', () => {
    expect(vodThumbUrl('https://cdn/thumb-%{width}x%{height}.jpg')).toBe(
      'https://cdn/thumb-440x248.jpg',
    );
    expect(vodThumbUrl('https://cdn/thumb-%{width}x%{height}.jpg', 160, 90)).toBe(
      'https://cdn/thumb-160x90.jpg',
    );
  });

  it('falls back for a missing url', () => {
    expect(vodThumbUrl(undefined)).toBe(VOD_FALLBACK_THUMB);
    expect(vodThumbUrl('')).toBe(VOD_FALLBACK_THUMB);
  });

  it('falls back for the still-processing placeholder', () => {
    // What Twitch returns for a VOD whose broadcast is still recording.
    expect(vodThumbUrl('https://vod-secure.twitch.tv/_404/404_processing_440x248.png')).toBe(
      VOD_FALLBACK_THUMB,
    );
  });

  it('passes a normal url through untouched', () => {
    const url = 'https://static-cdn.jtvnw.net/cf_vods/abc//thumb/thumb0-440x248.jpg';
    expect(vodThumbUrl(url)).toBe(url);
  });
});
