// The gallery's date orders and the detail line's date are Rust's:
// `standing.added[key].order` is each badge's place in the one Newest First
// order the phone wall and the site share (services/badge_index.rs), and
// `label` is its date as every surface words it. This file only looks them up,
// so no comparator here carries date logic of its own.
import type { AddedDate, BadgeStanding } from '../../services/badgeStanding';

/** The standing's `added` map, or nothing while the standing is still loading. */
export type AddedMap = Record<string, AddedDate> | null | undefined;

export interface BadgeRef {
  set_id: string;
  id: string;
}

/** The standing's own key for a badge: `set_id/version`, Twitch's case. */
export function addedKey(setId: string, version: string): string {
  return `${setId}/${version}`;
}

/** The key every surface settles ties with: lowercased `<set>-v<version>`. */
function tieKey(ref: BadgeRef): string {
  return `${ref.set_id}-v${ref.id}`.toLowerCase();
}

/** Code-unit order, never `localeCompare`: Rust and the site order `_` and `-` the same way, a locale compare does not. */
function byCodeUnit(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

/**
 * Newest First by Rust's place. A badge with no place (the standing is still
 * loading, or the badge list and the standing raced) sorts first, as an
 * undated badge does, and those settle by the lowercased key.
 */
export function compareNewest(added: AddedMap, a: BadgeRef, b: BadgeRef): number {
  const orderA = added?.[addedKey(a.set_id, a.id)]?.order;
  const orderB = added?.[addedKey(b.set_id, b.id)]?.order;
  if (orderA !== undefined && orderB !== undefined && orderA !== orderB) return orderA - orderB;
  if ((orderA === undefined) !== (orderB === undefined)) return orderA === undefined ? -1 : 1;
  return byCodeUnit(tieKey(a), tieKey(b));
}

/** The label for one badge, only ever from the standing; null when it has none. */
export function addedLabelFor(added: AddedMap, setId: string, version: string): string | null {
  return added?.[addedKey(setId, version)]?.label ?? null;
}

/**
 * The last standing this run answered, only when it is for `login` (compared
 * without case; signed out matches signed out). A standing is per account: after
 * an account switch the previous account's owned and missing badges must never
 * be painted, even for a moment.
 */
export function seedStanding(last: BadgeStanding | null, login: string | null | undefined): BadgeStanding | null {
  if (!last) return null;
  return (last.login ?? null)?.toLowerCase() === (login ?? null)?.toLowerCase() ? last : null;
}

/**
 * Applies only the newest request's answer. A slower, older read that resolves
 * after a newer one is dropped, so an older order never replaces a newer one.
 * A rejected request rejects its own promise and applies nothing.
 */
export function latestOnly<T>(apply: (value: T) => void): (request: Promise<T>) => Promise<void> {
  let newest = 0;
  return async (request) => {
    const mine = ++newest;
    const value = await request;
    if (mine === newest) apply(value);
  };
}
