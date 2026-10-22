// Which mod log entries offer to undo themselves: Unban on a ban, Lift timeout
// on a timeout still running. Only the newest ban, timeout or lift for a
// chatter in a channel counts, so a later unban (or a later ban that carries
// its own button) hides the older entry's action.

import type { ModLogEvent } from '../../types';

export type LiftKind = 'unban' | 'untimeout';

const RESTRICTS = new Set(['ban', 'timeout']);
const LIFTS = new Set(['unban', 'untimeout', 'approve_unban_request']);

function stamp(log: ModLogEvent): number {
  const t = Date.parse(log.timestamp);
  return Number.isNaN(t) ? 0 : t;
}

/** When a timeout entry runs out, or null when it has no length. */
export function timeoutEndsAt(log: ModLogEvent): number | null {
  if (log.action.toLowerCase() !== 'timeout' || log.duration == null || !(log.duration > 0)) return null;
  return stamp(log) + log.duration * 1000;
}

/**
 * The entries that offer a lift, by id. `canModerate` says whether the viewer
 * moderates an entry's channel (its lowercased `channel` key); entries
 * without a target user id never qualify, since the lift is addressed by it.
 */
export function liftActions(
  logs: readonly ModLogEvent[],
  nowMs: number,
  canModerate: (channel: string) => boolean,
): Map<string, LiftKind> {
  const latest = new Map<string, { log: ModLogEvent; at: number; lift: boolean }>();
  for (const log of logs) {
    const action = log.action.toLowerCase();
    const restricts = RESTRICTS.has(action);
    if (!restricts && !LIFTS.has(action)) continue;
    if (!log.target_user_id || !log.channel) continue;
    const key = `${log.channel.toLowerCase()}|${log.target_user_id}`;
    const at = stamp(log);
    const prev = latest.get(key);
    // On a tie the lift wins: nothing is left to undo.
    if (!prev || at > prev.at || (at === prev.at && !restricts && !prev.lift)) {
      latest.set(key, { log, at, lift: !restricts });
    }
  }
  const out = new Map<string, LiftKind>();
  for (const { log, lift } of latest.values()) {
    if (lift || !canModerate(log.channel!.toLowerCase())) continue;
    if (log.action.toLowerCase() === 'ban') {
      out.set(log.id, 'unban');
    } else {
      const ends = timeoutEndsAt(log);
      if (ends != null && ends > nowMs) out.set(log.id, 'untimeout');
    }
  }
  return out;
}

/** The soonest a shown Lift timeout runs out, so the view can drop it then. */
export function nextLiftExpiry(logs: readonly ModLogEvent[], lifts: ReadonlyMap<string, LiftKind>): number | null {
  let soonest: number | null = null;
  for (const log of logs) {
    if (lifts.get(log.id) !== 'untimeout') continue;
    const ends = timeoutEndsAt(log);
    if (ends != null && (soonest == null || ends < soonest)) soonest = ends;
  }
  return soonest;
}

/** Whether the viewer's own badges in a channel make them a moderator there. */
export function badgesModerate(badges: string | null | undefined): boolean {
  return !!badges && /(?:^|,)(?:moderator|broadcaster)\//.test(badges);
}
