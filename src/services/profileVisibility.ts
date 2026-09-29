// What a member can hide from their public profile, one switch per stat, and
// the rules for reading and saving those choices. The same list drives the app's
// profile and Settings, the website's page and account settings, and the site's
// server, which removes a hidden stat from what it serves.
//
// Stored in `user_profile_prefs.hidden_sections`. Older apps saved one key per
// SECTION (roast, twitch, lifetime, emotes, accolades, views). Those keys still
// work: a section key on its own hides the whole section. Once any stat of a
// section is saved on its own, the section key only tells older apps (which know
// nothing finer) to hide the section, so a viewer on an old version hides more
// than was asked, never less.

export type ProfileStatKey =
  | 'hours'
  | 'roast_line'
  | 'twitch_age'
  | 'followers'
  | 'account_type'
  | 'messages'
  | 'streams'
  | 'favorite_channel'
  | 'cosmetics'
  | 'member_rank'
  | 'emotes'
  | 'accolades'
  | 'views'
  | 'joined'
  | 'relics'
  | 'badge_collection'
  | 'paints';

export interface ProfileVisibilityGroup {
  label: string;
  /** The section key older apps saved for this group, when there was one. */
  sectionKey: string | null;
  items: Array<{ key: ProfileStatKey; label: string }>;
}

export const PROFILE_VISIBILITY_GROUPS: ProfileVisibilityGroup[] = [
  {
    label: 'Hours watched',
    sectionKey: 'roast',
    items: [
      { key: 'hours', label: 'Hours watched' },
      { key: 'roast_line', label: 'The line about your hours' },
    ],
  },
  {
    label: 'Twitch',
    sectionKey: 'twitch',
    items: [
      { key: 'twitch_age', label: 'Twitch account age' },
      { key: 'followers', label: 'Followers' },
      { key: 'account_type', label: 'Account type' },
    ],
  },
  {
    label: 'Lifetime',
    sectionKey: 'lifetime',
    items: [
      { key: 'messages', label: 'Messages sent' },
      { key: 'streams', label: 'Streams watched' },
      { key: 'favorite_channel', label: 'Favorite channel' },
      { key: 'cosmetics', label: 'Cosmetics count' },
      { key: 'member_rank', label: 'Member rank' },
    ],
  },
  { label: 'Top emotes', sectionKey: 'emotes', items: [{ key: 'emotes', label: 'Top emotes' }] },
  { label: 'Accolades', sectionKey: 'accolades', items: [{ key: 'accolades', label: 'Accolades' }] },
  { label: 'Profile views', sectionKey: 'views', items: [{ key: 'views', label: 'Profile views' }] },
  {
    label: 'Collection',
    sectionKey: null,
    items: [
      { key: 'joined', label: 'When you joined StreamNook' },
      { key: 'relics', label: 'Relics' },
      { key: 'badge_collection', label: 'Badge collection' },
      { key: 'paints', label: '7TV paints' },
    ],
  },
];

const SECTION_OF = new Map<string, string>();
const STATS_OF = new Map<string, ProfileStatKey[]>();
for (const g of PROFILE_VISIBILITY_GROUPS) {
  if (!g.sectionKey) continue;
  STATS_OF.set(
    g.sectionKey,
    g.items.map((i) => i.key),
  );
  for (const i of g.items) if (i.key !== g.sectionKey) SECTION_OF.set(i.key, g.sectionKey);
}

/** Every key `hidden_sections` may hold: each stat, and each older section key. */
export const PROFILE_VISIBILITY_KEYS: ReadonlySet<string> = new Set([
  ...PROFILE_VISIBILITY_GROUPS.flatMap((g) => g.items.map((i) => i.key)),
  ...STATS_OF.keys(),
]);

/** Whether a stat is hidden, given what `hidden_sections` holds. */
export function isStatHidden(stored: readonly string[], stat: ProfileStatKey): boolean {
  if (stored.includes(stat)) return true;
  const section = SECTION_OF.get(stat);
  if (!section || !stored.includes(section)) return false;
  // The section key hides the whole section only when none of its stats were
  // saved on their own (the way older apps saved it).
  return !(STATS_OF.get(section) ?? []).some((k) => stored.includes(k));
}

/** Every hidden stat, from what `hidden_sections` holds. */
export function hiddenStats(stored: readonly string[]): Set<ProfileStatKey> {
  const out = new Set<ProfileStatKey>();
  for (const g of PROFILE_VISIBILITY_GROUPS) for (const i of g.items) if (isStatHidden(stored, i.key)) out.add(i.key);
  return out;
}

/** What to save for a set of hidden stats: the stats, plus the older section
 *  key for any section with a stat hidden, so older apps hide that section. */
export function storedKeys(hidden: ReadonlySet<ProfileStatKey>): string[] {
  const out = new Set<string>(hidden);
  for (const g of PROFILE_VISIBILITY_GROUPS) {
    if (g.sectionKey && g.items.some((i) => hidden.has(i.key))) out.add(g.sectionKey);
  }
  return [...out];
}

/** The stored list after one stat is switched. */
export function toggleStat(stored: readonly string[], stat: ProfileStatKey): string[] {
  const hidden = hiddenStats(stored);
  if (hidden.has(stat)) hidden.delete(stat);
  else hidden.add(stat);
  return storedKeys(hidden);
}
