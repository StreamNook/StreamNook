import { describe, expect, it } from 'vitest';
import type { RewardAvailability } from '../types';
import { formatCostShort, formatCountdown, rewardKind, rewardStatusText, rewardTakesText } from './rewardStatus';

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

describe('formatCostShort', () => {
  it('keeps small costs whole and shortens thousands and millions', () => {
    expect(formatCostShort(950)).toBe('950');
    expect(formatCostShort(3_000)).toBe('3k');
    expect(formatCostShort(2_500)).toBe('2.5k');
    expect(formatCostShort(150_000)).toBe('150k');
    expect(formatCostShort(20_000_000)).toBe('20M');
    expect(formatCostShort(1_200_000)).toBe('1.2M');
  });
});

describe('rewardKind', () => {
  it('routes built-in rewards by type, never by title', () => {
    expect(rewardKind({ reward_type: 'SEND_HIGHLIGHTED_MESSAGE', is_user_input_required: true })).toBe('highlight');
    expect(rewardKind({ reward_type: 'SINGLE_MESSAGE_BYPASS_SUB_MODE', is_user_input_required: true })).toBe('sub_message');
    expect(rewardKind({ reward_type: 'RANDOM_SUB_EMOTE_UNLOCK', is_user_input_required: false })).toBe('random_emote');
    expect(rewardKind({ reward_type: 'CHOSEN_SUB_EMOTE_UNLOCK', is_user_input_required: false })).toBe('choose_emote');
    expect(rewardKind({ reward_type: 'CHOSEN_MODIFIED_SUB_EMOTE_UNLOCK', is_user_input_required: false })).toBe('modify_emote');
  });

  it('treats custom rewards as input or standard', () => {
    expect(rewardKind({ reward_type: null, is_user_input_required: true })).toBe('input');
    expect(rewardKind({ reward_type: null, is_user_input_required: false })).toBe('standard');
  });
});

describe('rewardTakesText', () => {
  it('is true for the message rewards and custom input rewards only', () => {
    expect(rewardTakesText('highlight')).toBe(true);
    expect(rewardTakesText('sub_message')).toBe(true);
    expect(rewardTakesText('input')).toBe(true);
    expect(rewardTakesText('standard')).toBe(false);
    expect(rewardTakesText('random_emote')).toBe(false);
  });
});
