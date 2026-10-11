// The name a chat row shows for its author. Rust decides it once per row
// (`services::chatter_name`, stamped as `metadata.shown_name`) so a plain
// message, a sub, a watch streak and a raid all name the same person the same
// way; the page only lays the viewer's own nickname override over it, because
// the override map is live page state.
//
// Rows that never pass the Rust rule engine (VOD replay, a raw IRC string the
// backend could not parse, an optimistic local row) carry no stamp and show
// their display name, the value they showed before the stamp existed.
//
// Nicknames are saved and looked up under ONE key, built here for both sides:
// the bare user id for Twitch, `provider:id` for every other platform (the
// cosmetics key space). Never a login or a display name: a localized name is
// not an identity, and a chatter can rename.
import { getDisplayedName } from './userChatOverrides';
import { chatterId, chatterProvider, cosmeticsKey } from './chatterIdentity';
import type { UserChatOverride } from '../types';
import type { ProviderId } from '../types/providers';

export interface NamedRow {
  username: string;
  displayName?: string;
  tags: Map<string, string>;
  metadata?: { shown_name?: string };
}

/** The key a chatter's nickname and color override are saved and read under.
 *  Empty when there is no id: nothing can be saved or found without one. */
export function overrideKey(userId: string | null | undefined, provider: ProviderId = 'twitch'): string {
  return userId ? cosmeticsKey(userId, provider) : '';
}

/** The override key for a chat row's author (Twitch `user-id` tag, or the
 *  provider's own user id). */
export function rowOverrideKey(row: { provider?: ProviderId; tags: Map<string, string>; providerUserId?: string }): string {
  return overrideKey(chatterId(row), chatterProvider(row));
}

/** The author name before the viewer's nickname override. */
export function rowBaseName(row: NamedRow): string {
  return row.metadata?.shown_name || row.tags.get('display-name') || row.displayName || row.username;
}

/** Every name a platform's own event text may open with for this row's author
 *  (Twitch's `system-msg` uses the display name, or the login when the notice
 *  carries no localized name). */
export function rowSenderNames(row: NamedRow): string[] {
  const names = [row.tags.get('display-name'), row.displayName, row.username, row.tags.get('login')];
  return names.filter((n, i): n is string => !!n && names.indexOf(n) === i);
}

/** `text` without a leading author name (any of `names`, case-insensitive). */
export function stripLeadingName(text: string, names: readonly (string | undefined)[]): string {
  const lower = text.toLowerCase();
  // Longest first, so "mochi_panda" is not cut down by a shorter alias.
  for (const n of [...names].filter((x): x is string => !!x).sort((a, b) => b.length - a.length)) {
    if (lower.startsWith(`${n.toLowerCase()} `)) return text.slice(n.length).trimStart();
  }
  return text;
}

const escapeRegExp = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/**
 * The platform's event text with the author's name swapped for the name the row
 * shows, so `{default}` in a custom wording names the chatter the same way the
 * card does (nickname included) instead of by raw login. The first whole-name
 * match in any script is replaced ("12 raiders from X have joined!" names the
 * author mid-sentence).
 */
export function withShownSender(text: string, row: NamedRow, shown: string): string {
  const names = rowSenderNames(row).sort((a, b) => b.length - a.length).map(escapeRegExp);
  if (names.length === 0) return text;
  const pattern = new RegExp(`(?<![\\p{L}\\p{N}_])(?:${names.join('|')})(?![\\p{L}\\p{N}_])`, 'iu');
  return text.replace(pattern, () => shown);
}

/**
 * The author name a row renders: the viewer's nickname for this chatter when
 * one is set, else the stamped name. `key` comes from `rowOverrideKey`.
 */
export function rowShownName(
  row: NamedRow,
  key: string | null | undefined,
  overrides: Record<string, UserChatOverride> | undefined,
): string {
  return getDisplayedName(key, rowBaseName(row), overrides);
}
