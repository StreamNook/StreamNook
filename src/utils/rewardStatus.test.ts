import { describe, expect, it } from 'vitest';
import type { RewardAvailability } from '../types';
import { formatCountdown, rewardStatusText } from './rewardStatus';

const blocked = (over: Partial<RewardAvailability>): RewardAvailability => ({
  redeemable: false,
  reason: 'paused',
  cooldown_until_ms: null,
  affordable: true,
  label: 'Paused',
  detail: 'Paused by the streamer',
  ...over,
});

describe('formatCountdown', () => {
  it('shows minutes and seconds under an hour', () => {
    expect(formatCountdown(252_000)).toBe('4:12');
    expect(formatCountdown(5_000)).toBe('0:05');
  });

  it('adds hours past an hour', () => {
    expect(formatCountdown(3_849_000)).toBe('1:04:09');
  });

  it('rounds a partial second up and never goes negative', () => {
    expect(formatCountdown(100)).toBe('0:01');
    expect(formatCountdown(-5_000)).toBe('0:00');
  });
});

describe('rewardStatusText', () => {
  it('is null for a redeemable reward', () => {
    expect(rewardStatusText(blocked({ redeemable: true, reason: null, label: null, detail: null }), 0, 'label')).toBeNull();
  });

  it('picks the short or the full words', () => {
    const a = blocked({});
    expect(rewardStatusText(a, 0, 'label')).toBe('Paused');
    expect(rewardStatusText(a, 0, 'detail')).toBe('Paused by the streamer');
  });

  it('adds the time left to a running cooldown', () => {
    const a = blocked({ reason: 'on_cooldown', cooldown_until_ms: 300_000, label: 'On cooldown', detail: 'On cooldown' });
    expect(rewardStatusText(a, 48_000, 'label')).toBe('On cooldown · 4:12');
  });

  it('falls back to the label when there is no detail', () => {
    expect(rewardStatusText(blocked({ detail: null }), 0, 'detail')).toBe('Paused');
  });
});
