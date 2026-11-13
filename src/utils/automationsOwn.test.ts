import { readFileSync } from 'node:fs';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import {
  actionOf,
  automationsApi,
  blankRule,
  duplicateRule,
  isEdgeKind,
  isOutward,
  isOwnEventKind,
  previewSample,
  rewardLabel,
  triggerOf,
  withTrigger,
} from './automations';

const owner = { provider: 'twitch', id: '100', login: 'me', name: 'me' };

describe('P3 triggers and actions', () => {
  beforeEach(() => invoke.mockReset());

  it('builds each new trigger and action in the shape Rust reads', () => {
    expect(triggerOf('follow')).toEqual({ kind: 'follow' });
    expect(triggerOf('reward_redeemed')).toEqual({ kind: 'reward_redeemed', reward_id: null });
    expect(triggerOf('live_change')).toEqual({ kind: 'live_change', online: true });
    expect(actionOf('speak')).toMatchObject({ kind: 'speak', text: '', on_stream: false });
    expect(actionOf('highlight')).toMatchObject({ kind: 'highlight', color: '#f97316' });
    expect(isOutward(actionOf('speak'))).toBe(false);
    expect(isOutward(actionOf('highlight'))).toBe(false);
  });

  it('knows which triggers are not chat lines', () => {
    for (const k of ['follow', 'reward_redeemed', 'poll_started', 'prediction_started'] as const) {
      expect(isOwnEventKind(k)).toBe(true);
      expect(isEdgeKind(k)).toBe(true);
    }
    expect(isOwnEventKind('live_change')).toBe(false);
    expect(isEdgeKind('live_change')).toBe(true);
    expect(isEdgeKind('message')).toBe(false);
  });

  it('turns a reply into a post and forces your channel when a rule moves to an own-channel event', () => {
    const rule = blankRule(owner);
    expect(rule.actions[0].kind).toBe('reply');
    const next = withTrigger(rule, 'follow');
    expect(next.when).toEqual({ kind: 'follow' });
    expect(next.only_if.channels).toEqual({ kind: 'own' });
    expect(next.actions[0].kind).toBe('send');
  });

  it('keeps the channel list through a live change and clears the people lists', () => {
    const rule = blankRule(owner);
    rule.only_if.skip_users = ['bot'];
    const live = withTrigger(
      { ...rule, only_if: { ...rule.only_if, channels: { kind: 'these', list: [{ id: 'c-1', key: 'twitch:a', room_id: '1', name: 'a' }] } } },
      'live_change',
    );
    expect(live.only_if.channels).toMatchObject({ kind: 'these', list: [{ name: 'a' }] });
    expect(live.only_if.skip_users).toEqual([]);
    expect(live.only_if.only_live).toBe(false);
    const fresh = withTrigger(blankRule(owner), 'live_change');
    expect(fresh.only_if.channels).toEqual({ kind: 'these', list: [] });
  });

  it('gives a duplicate new ids for speak and highlight actions too', () => {
    const rule = { ...blankRule(owner), actions: [actionOf('speak'), actionOf('highlight')] };
    const copy = duplicateRule(rule);
    expect(copy.actions.map((a) => a.kind)).toEqual(['speak', 'highlight']);
    expect(copy.actions[0].id).not.toBe(rule.actions[0].id);
  });

  it('previews a P3 rule with no chat text', () => {
    const rule = withTrigger(blankRule(owner), 'follow');
    expect(previewSample(rule).text).toBe('');
  });

  it('labels a reward with its price and says when it is off', () => {
    expect(rewardLabel({ id: 'a', title: 'Hydrate', cost: 1500, enabled: true, needs_input: false })).toBe('Hydrate (1,500 points)');
    expect(rewardLabel({ id: 'a', title: ' ', cost: 5, enabled: false, needs_input: false })).toBe('Untitled reward (5 points, off)');
  });

  it('asks for the owner rewards by the literal command name', async () => {
    invoke.mockResolvedValue([]);
    await automationsApi.ownRewards('100');
    await automationsApi.ownRewards(null);
    expect(invoke).toHaveBeenNthCalledWith(1, 'automations_own_rewards', { ownerId: '100' });
    expect(invoke).toHaveBeenNthCalledWith(2, 'automations_own_rewards', { ownerId: null });
  });

  it('keeps an automation highlight from flashing the window title', () => {
    // Source pin: the flash effect returns early for the `automation` kind (an automation stamp tints only).
    const source = readFileSync(new URL('../components/ChatMessage.tsx', import.meta.url), 'utf8');
    const flash = source.slice(source.indexOf('Window-title flash.'));
    expect(flash.slice(0, flash.indexOf('flashTitle('))).toContain("highlight?.kind === 'automation'");
  });
});
