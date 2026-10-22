import { describe, expect, it } from 'vitest';

import type { ModLogEvent } from '../../types';
import { badgesModerate, liftActions, nextLiftExpiry } from './modLogLift';

const T0 = Date.parse('2026-10-21T12:00:00Z');
const at = (s: number) => new Date(T0 + s * 1000).toISOString();

let seq = 0;
function entry(action: string, s: number, extra: Partial<ModLogEvent> = {}): ModLogEvent {
  return {
    id: `e${++seq}`,
    action,
    timestamp: at(s),
    moderator_name: 'mod',
    target_user_id: '42',
    target_user_name: 'Pest',
    channel: 'chan',
    ...extra,
  };
}

const mods = (ch: string) => ch === 'chan';

describe('liftActions', () => {
  it('offers Unban on a ban and Lift timeout on a running timeout', () => {
    const ban = entry('ban', 0);
    const to = entry('timeout', 0, { target_user_id: '7', duration: 600 });
    const lifts = liftActions([ban, to], T0 + 1000, mods);
    expect(lifts.get(ban.id)).toBe('unban');
    expect(lifts.get(to.id)).toBe('untimeout');
  });

  it('hides a timeout once it has run out', () => {
    const to = entry('timeout', 0, { duration: 60 });
    expect(liftActions([to], T0 + 59_000, mods).has(to.id)).toBe(true);
    expect(liftActions([to], T0 + 60_000, mods).has(to.id)).toBe(false);
  });

  it('hides a timeout with no length, which cannot be known to run', () => {
    const to = entry('timeout', 0);
    expect(liftActions([to], T0, mods).size).toBe(0);
  });

  it('hides the action once a later unban or untimeout is logged', () => {
    const ban = entry('ban', 0);
    const unban = entry('unban', 30);
    expect(liftActions([unban, ban], T0 + 60_000, mods).size).toBe(0);
    const to = entry('timeout', 100, { duration: 600 });
    const untimeout = entry('untimeout', 120);
    expect(liftActions([untimeout, to], T0 + 130_000, mods).size).toBe(0);
    const approved = entry('approve_unban_request', 200);
    expect(liftActions([approved, entry('ban', 150)], T0 + 300_000, mods).size).toBe(0);
  });

  it('keeps the action when the unban came before the ban', () => {
    const unban = entry('unban', 0);
    const ban = entry('ban', 30);
    expect(liftActions([ban, unban], T0 + 60_000, mods).get(ban.id)).toBe('unban');
  });

  it('only the newest restriction for a chatter carries the action', () => {
    const to = entry('timeout', 0, { duration: 3600 });
    const ban = entry('ban', 60);
    const lifts = liftActions([ban, to], T0 + 120_000, mods);
    expect(lifts.get(ban.id)).toBe('unban');
    expect(lifts.has(to.id)).toBe(false);
  });

  it('a lift logged in the same second wins the tie', () => {
    const ban = entry('ban', 0);
    const unban = entry('unban', 0);
    expect(liftActions([ban, unban], T0 + 1000, mods).size).toBe(0);
  });

  it('keeps chatters and channels apart', () => {
    const ban = entry('ban', 0);
    const otherUser = entry('unban', 30, { target_user_id: '99' });
    const otherChan = entry('unban', 30, { channel: 'elsewhere' });
    expect(liftActions([otherChan, otherUser, ban], T0 + 60_000, mods).get(ban.id)).toBe('unban');
  });

  it('offers nothing where the viewer does not moderate', () => {
    const ban = entry('ban', 0, { channel: 'elsewhere' });
    expect(liftActions([ban], T0, mods).size).toBe(0);
  });

  it('offers nothing without a target user id', () => {
    const ban = entry('ban', 0, { target_user_id: undefined });
    expect(liftActions([ban], T0, mods).size).toBe(0);
  });

  it('ignores other actions', () => {
    const del = entry('delete', 0);
    const removed = entry('removed', 0);
    expect(liftActions([del, removed], T0, mods).size).toBe(0);
  });
});

describe('nextLiftExpiry', () => {
  it('is the soonest end among the shown Lift timeouts', () => {
    const a = entry('timeout', 0, { target_user_id: '1', duration: 600 });
    const b = entry('timeout', 0, { target_user_id: '2', duration: 60 });
    const ban = entry('ban', 0, { target_user_id: '3' });
    const logs = [a, b, ban];
    expect(nextLiftExpiry(logs, liftActions(logs, T0, mods))).toBe(T0 + 60_000);
  });

  it('is null with no timeout shown', () => {
    const ban = entry('ban', 0);
    expect(nextLiftExpiry([ban], liftActions([ban], T0, mods))).toBeNull();
  });
});

describe('badgesModerate', () => {
  it('reads moderator and broadcaster badges', () => {
    expect(badgesModerate('moderator/1,subscriber/12')).toBe(true);
    expect(badgesModerate('subscriber/12,broadcaster/1')).toBe(true);
    expect(badgesModerate('subscriber/12,vip/1')).toBe(false);
    expect(badgesModerate('')).toBe(false);
    expect(badgesModerate(null)).toBe(false);
  });
});
