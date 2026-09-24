import { invoke } from '@tauri-apps/api/core';

/**
 * Thin wrapper over the Rust cross-platform identity cache.
 *
 * Rust owns the cache and the network, so the main window, every MultiChat
 * popout and the phone all share one copy. That matters twice over: it keeps a
 * popout from paying its own lookups, and it means a "nobody has claimed this"
 * answer outlives any single window's chat store, which is the answer for almost
 * every chatter.
 */

/**
 * Which StreamNook member, if any, is behind each chat key.
 *
 * Keys follow the chat convention (`kick:12345`, `youtube:UCabc`); bare Twitch
 * ids need no translation and are ignored. Unclaimed keys are simply absent
 * from the result, which is the ordinary case rather than an error.
 */
export interface MemberResolution {
  /** Chat key -> the member's Twitch id, for every key that has a claim. */
  resolved: Record<string, string>;
  /**
   * Keys that were NOT answered: the request failed, the circuit breaker is
   * open, or the batch was over the cap. Different from "no claim" and must be
   * asked about again. A key in neither list has no claim.
   */
  unresolved: string[];
}

export async function resolveMemberIds(keys: string[]): Promise<MemberResolution> {
  if (keys.length === 0) return { resolved: {}, unresolved: [] };
  let raw: unknown;
  try {
    raw = await invoke<unknown>('resolve_member_ids', { keys });
  } catch {
    // The call itself failed, so nothing was answered: every key goes back to be
    // asked again rather than being taken for "not a member".
    return { resolved: {}, unresolved: [...keys] };
  }
  return normaliseResolution(raw, keys);
}

/**
 * Accept whatever shape the backend answered with.
 *
 * The frontend can be newer than the Rust it is talking to — a hot reload in
 * development, or a partial update — and an older backend answered with a bare
 * `{ key: memberId }` map. Reading that as the new shape would throw inside the
 * batch and lose every answer in it, so each shape is recognised explicitly and
 * anything unrecognisable counts as not answered.
 */
export function normaliseResolution(raw: unknown, keys: string[]): MemberResolution {
  if (raw && typeof raw === 'object' && !Array.isArray(raw)) {
    const r = raw as { resolved?: unknown; unresolved?: unknown };
    if (r.resolved && typeof r.resolved === 'object' && !Array.isArray(r.resolved)) {
      return {
        resolved: r.resolved as Record<string, string>,
        unresolved: Array.isArray(r.unresolved)
          ? r.unresolved.filter((k): k is string => typeof k === 'string')
          : [],
      };
    }
    // The older bare map: every value a member id, and nothing reported as
    // unanswered because that version could not say.
    const entries = Object.entries(raw as Record<string, unknown>);
    if (entries.every(([, v]) => typeof v === 'string')) {
      return { resolved: Object.fromEntries(entries) as Record<string, string>, unresolved: [] };
    }
  }
  return { resolved: {}, unresolved: [...keys] };
}

/**
 * Forget every cached claim.
 *
 * Called after the local user links or unlinks a platform account. Their own
 * claim just changed, and a "nobody" cached moments earlier would otherwise keep
 * their badge off their own messages until the app restarted.
 */
export async function invalidateMemberAliases(): Promise<void> {
  try {
    await invoke<void>('invalidate_member_aliases');
  } catch {
    /* best effort; the TTL still recovers it */
  }
}
