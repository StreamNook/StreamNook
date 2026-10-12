import type { ThirdPartyBadge } from '../services/badgeService';

/** The chat_design fields that decide which badges a chatter shows. */
export interface BadgeDisplaySettings {
  show_badges?: boolean;
  show_third_party_badges?: boolean;
  hidden_badge_providers?: string[];
}

export interface VisibleChatterBadges<T, S> {
  twitch: readonly T[];
  seventv: S | null;
  thirdParty: readonly ThirdPartyBadge[];
  /** Whether the StreamNook badge may show (membership is the caller's check). */
  streamNook: boolean;
}

const NONE: never[] = [];

/**
 * The visibility key of a Homies custom badge (a viewer can hide it apart from
 * the Homies role and staff badges). It is also that badge's wear category, so
 * the Rust index's `set_id` for it carries the same string.
 */
export const HOMIES_CUSTOM_SOURCE = 'homies-custom';

/**
 * The key a third-party badge is shown or hidden by in the badge services
 * setting (`hidden_badge_providers`): its provider, except a Homies custom
 * badge, which has its own entry. The two Homies entries are independent:
 * hiding Homies leaves custom badges up.
 */
export function badgeSourceKey(b: { provider?: string; id?: string; category?: string }): string {
  const provider = (b.provider ?? '').toLowerCase();
  if (
    provider === 'homies' &&
    (b.category === HOMIES_CUSTOM_SOURCE || b.id === HOMIES_CUSTOM_SOURCE || b.id === `homies:${HOMIES_CUSTOM_SOURCE}`)
  ) {
    return HOMIES_CUSTOM_SOURCE;
  }
  return provider;
}

/**
 * Applies the badge settings to a chatter's badges. Every surface that draws a
 * chatter (chat rows, the pinned message) filters through this, so a hidden
 * provider is hidden everywhere. A third-party badge is matched by
 * `badgeSourceKey`, so Homies custom badges toggle on their own. Inputs come back unchanged when nothing is
 * filtered out, so memoized readers keep their references.
 */
export function visibleChatterBadges<T, S>(
  design: BadgeDisplaySettings | undefined,
  twitch: readonly T[],
  seventv: S | null | undefined,
  thirdParty: readonly ThirdPartyBadge[],
): VisibleChatterBadges<T, S> {
  const badgesOn = design?.show_badges !== false;
  const thirdPartyOn = design?.show_third_party_badges !== false;
  const hidden = design?.hidden_badge_providers ?? NONE;
  const isHidden = (src?: string) => hidden.includes((src || '').toLowerCase());
  const keptThirdParty = !thirdPartyOn
    ? NONE
    : hidden.length === 0
      ? thirdParty
      : thirdParty.filter((b) => !isHidden(badgeSourceKey(b)));
  return {
    twitch: badgesOn ? twitch : NONE,
    seventv: thirdPartyOn && !isHidden('7tv') ? (seventv ?? null) : null,
    thirdParty: keptThirdParty,
    streamNook: thirdPartyOn && !isHidden('streamnook'),
  };
}

/** A badge's provider as one key, whatever its spelling (`BTTV` and `bttv`). */
const providerKey = (provider?: string) => (provider ?? '').toLowerCase();

/** The Homies custom badge's id; its loadout key is `homies:homies-custom`. */
const HOMIES_CUSTOM_ID = 'homies-custom';

/**
 * A badge's wear category: what its provider treats as one separate thing. A
 * chatter wears one badge per category. Homies has three (the custom badge,
 * the role list, the staff list), FFZ one per slot, every other provider one.
 * The Rust index and the Identity API name it in `category` (the Rust
 * `set_id`); without one, the Homies custom badge still counts apart and
 * anything else counts as its provider's main category.
 */
export function wearCategory(b: { provider?: string; id?: string; category?: string }): string {
  const provider = providerKey(b.provider);
  if (b.category) return `${provider}:${b.category.toLowerCase()}`;
  if (provider === 'homies' && b.id === HOMIES_CUSTOM_ID) return `homies:${HOMIES_CUSTOM_ID}`;
  return `${provider}:${provider}`;
}

/** The category of a loadout key (`<provider>:<id>`) that no badge list names. */
export function wearCategoryOfKey(key: string): string {
  const sep = key.indexOf(':');
  return wearCategory({ provider: key.slice(0, sep), id: key.slice(sep + 1) });
}

/**
 * A chatter WEARS at most one badge per category (see `wearCategory`). Keeps
 * the first badge of each category, so each provider's badges must arrive
 * best-first: the Rust index and the Identity API both answer that way, and
 * the BTTV Pro badge is placed after a member's other BetterTTV badge, as
 * BetterTTV draws them. Returns the input unchanged when nothing is dropped.
 */
export function onePerCategory<T extends { provider?: string; id?: string; category?: string }>(
  badges: readonly T[],
): readonly T[] {
  const seen = new Set<string>();
  const kept = badges.filter((b) => {
    const key = wearCategory(b);
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
  return kept.length === badges.length ? badges : kept;
}

/**
 * Turns one loadout key on or off. Switching a badge on switches off any
 * other badge of the same category, because chat shows one per category; keys
 * of other categories (the Twitch badge included) are kept. `categoryOf` maps
 * a key to its category, from the badge list the picker shows.
 */
export function toggleLoadoutBadge(
  badges: readonly string[],
  key: string,
  categoryOf: (key: string) => string = wearCategoryOfKey,
): string[] {
  if (badges.includes(key)) return badges.filter((k) => k !== key);
  const category = categoryOf(key);
  return [...badges.filter((k) => categoryOf(k) !== category), key];
}

/**
 * Twitch role badge sets, ranked as Rust's `chat_layout::twitch_badge_rank`
 * ranks them (lowest first): broadcaster, then staff, then moderator, then
 * VIP. Subscription, earned and global badges are not roles.
 */
const TWITCH_ROLE_RANK: Readonly<Record<string, number>> = {
  broadcaster: 0,
  staff: 1,
  admin: 1,
  global_mod: 1,
  lead_moderator: 2,
  moderator: 2,
  vip: 3,
};

/**
 * The one badge a compact one-line byline (the collapsed pin bar) shows: the
 * sender's highest Twitch role badge, or none. Takes resolved Twitch badges
 * keyed `set/version`; third-party badges never reach it. Picked by rank, not
 * position, so it holds whatever order the badges arrive in.
 */
export function pickRoleBadge<T extends { key: string }>(twitch: readonly T[]): T | null {
  let best: T | null = null;
  let bestRank = Infinity;
  for (const b of twitch) {
    const rank = TWITCH_ROLE_RANK[b.key.split('/')[0]];
    if (rank !== undefined && rank < bestRank) {
      best = b;
      bestRank = rank;
    }
  }
  return best;
}
