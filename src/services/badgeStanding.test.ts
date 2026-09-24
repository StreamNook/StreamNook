import { describe, expect, it } from 'vitest';
import {
  closesInLabel,
  earnChipHint,
  earnChipText,
  refetchDelay,
  timeLeftLabel,
  usdLabel,
  watchTimeLabel,
  windowStatusAt,
} from './badgeStanding';

const HOUR = 3_600_000;

describe('windowStatusAt', () => {
  it('reads open ends as unbounded', () => {
    expect(windowStatusAt([{ start_ms: null, end_ms: 10 }], 5)).toBe('available');
    expect(windowStatusAt([{ start_ms: 10, end_ms: null }], 5)).toBe('coming-soon');
    expect(windowStatusAt([{ start_ms: 10, end_ms: null }], 50)).toBe('available');
  });

  it('is not earnable in the gap between runs', () => {
    const runs = [
      { start_ms: 0, end_ms: 10 },
      { start_ms: 20, end_ms: 30 },
    ];
    expect(windowStatusAt(runs, 5)).toBe('available');
    expect(windowStatusAt(runs, 15)).toBe('coming-soon');
    expect(windowStatusAt(runs, 31)).toBe('expired');
  });

  it('has no status without a window', () => {
    expect(windowStatusAt(null)).toBeNull();
    expect(windowStatusAt([])).toBeNull();
  });
});

describe('refetchDelay', () => {
  it('never schedules past what setTimeout can hold', () => {
    expect(refetchDelay(1_000 + 60 * 24 * HOUR, 1_000)).toBe(6 * HOUR);
  });
  it('schedules nothing without a boundary', () => {
    expect(refetchDelay(null)).toBeNull();
  });
  it('re-asks shortly when a boundary has already passed', () => {
    expect(refetchDelay(0, 10_000)).toBe(1_000);
  });
  it('lands just after the boundary', () => {
    expect(refetchDelay(5_000 + HOUR, 5_000)).toBe(HOUR + 500);
  });
});

describe('earnChipText', () => {
  it('reads each kind of step as a short chip', () => {
    expect(earnChipText({ kind: 'watch', minutes: 30, days: null })).toBe('30 min');
    expect(earnChipText({ kind: 'watch', minutes: 60, days: null })).toBe('1 hr');
    expect(earnChipText({ kind: 'watch', minutes: 20, days: 3 })).toBe('20 min × 3d');
    expect(earnChipText({ kind: 'watch', minutes: null, days: null })).toBe('Watch');
    expect(earnChipText({ kind: 'subscribe', count: null })).toBe('Sub');
    expect(earnChipText({ kind: 'subscribe', count: 2 })).toBe('2 subs');
    expect(earnChipText({ kind: 'purchase', ticket: true })).toBe('Ticket');
    expect(earnChipText({ kind: 'purchase', ticket: false })).toBe('Paid');
    expect(earnChipText({ kind: 'other' })).toBe('Special');
  });

  it('spells the step out for the tooltip', () => {
    expect(earnChipHint({ kind: 'watch', minutes: 20, days: 3 })).toBe('Watch 20 min on 3 different days');
    expect(earnChipHint({ kind: 'purchase', ticket: true })).toBe('Granted with a ticket purchase');
  });
});

describe('timeLeftLabel', () => {
  it('counts down in days, then hours', () => {
    expect(timeLeftLabel(3 * 24 * HOUR, 0)).toBe('3d left');
    expect(timeLeftLabel(5 * HOUR, 0)).toBe('5h left');
    expect(timeLeftLabel(HOUR / 2, 0)).toBe('ends soon');
    expect(timeLeftLabel(null, 0)).toBe('');
  });
});

describe('closesInLabel', () => {
  it('finishes a sentence', () => {
    expect(closesInLabel(3 * 24 * HOUR, 0)).toBe('in 3d');
    expect(closesInLabel(5 * HOUR, 0)).toBe('in 5h');
    expect(closesInLabel(HOUR / 2, 0)).toBe('soon');
    expect(closesInLabel(null, 0)).toBe('');
  });
});

describe('catch-up labels', () => {
  it('prices subs in dollars', () => {
    expect(usdLabel(3 * 599)).toBe('$17.97');
    expect(usdLabel(0)).toBe('$0.00');
  });
  it('reads watch time at a glance', () => {
    expect(watchTimeLabel(45)).toBe('45m');
    expect(watchTimeLabel(120)).toBe('2h');
    expect(watchTimeLabel(380)).toBe('6h 20m');
    expect(watchTimeLabel(51 * 60)).toBe('2d 3h');
    expect(watchTimeLabel(72 * 60)).toBe('3d');
  });
});
