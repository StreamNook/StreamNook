// Single source of truth for the order badges render in on chat ROWS (in-app
// chat and the overlay renderer). The canonical order, left to right (badges sit
// before the username, so "last" = rightmost = adjacent to the name; YouTube rows
// put badges AFTER the name, where "last" is still the rightmost slot):
//
//   0. Source platform mark          - only on a row from a platform OTHER than
//      (chat only, merged feeds)       the one being watched, so the tier is
//                                      absent on every ordinary row. Leftmost
//                                      because it answers "which community is
//                                      this person in" before anything about
//                                      their standing in it.
//   1. Channel-contextual Twitch     - your standing in THIS channel: subscriber,
//      badges (chat only)              predictions/poll, bits, founder, etc. Dynamic.
//   2. Global Twitch badges          - your portable Twitch identity: partner,
//                                       prime/turbo, staff, etc.
//   3. 7TV badge
//   4. Third-party badges            - BTTV / FFZ / Chatterino / Homies / BTTV Pro
//                                       (any order among themselves)
//   5. StreamNook member badge       - who you are on StreamNook. Rightmost, next
//                                       to the name, where readers look first.
//
// Profiles follow the same order: a member card's worn-badge row, and the user
// card's badge panel, which groups badges by provider in that sequence.
// Chat surfaces order the tiers by laying their JSX blocks out in this sequence;
// this module owns the only piece that needs real logic: the split of a
// chatter's Twitch badges into the channel-contextual vs global tiers.

// Twitch badge SET ids that are scoped to the current channel (different image /
// meaning per channel) rather than global identity. Drives both badge ordering
// (these sort ahead of global Twitch badges) and image caching (channel-scoped
// badges are never cached, to avoid one channel's sub badge bleeding into another).
export const CHANNEL_SPECIFIC_TWITCH_BADGES = new Set([
  'subscriber',
  'bits',
  'sub-gifter',
  'sub-gift-leader',
  'founder',
  'hype-train',
  'predictions',
]);

/**
 * Twitch badge SETS that describe a standing in ONE channel (being its
 * broadcaster, mod or VIP there, subbed, cheered), never who someone is. They
 * are left out wherever a profile shows off a person's badges: everyone who
 * goes live has the broadcaster badge, and a sub badge is one channel's art.
 * A superset of CHANNEL_SPECIFIC_TWITCH_BADGES, which only drives ordering.
 */
export const CHANNEL_SCOPED_TWITCH_BADGE_SETS = new Set([
  'broadcaster',
  'moderator',
  'lead_moderator',
  'vip',
  'subscriber',
  'founder',
  'bits',
  'bits-leader',
  'sub-gifter',
  'sub-gift-leader',
  'artist-badge',
  'predictions',
  'hype-train',
  'clip-champ',
]);

/** True for a badge set that belongs to one channel, not to the person. */
export function isChannelScopedTwitchBadge(setId: string | null | undefined): boolean {
  return !!setId && CHANNEL_SCOPED_TWITCH_BADGE_SETS.has(setId);
}

/** True for a Twitch badge set scoped to the current channel (subscriber, poll, …). */
export function isChannelSpecificTwitchBadge(setId: string): boolean {
  return CHANNEL_SPECIFIC_TWITCH_BADGES.has(setId);
}

// Channel-contextual badges sort ahead of global ones; same-tier badges keep their
// original (Twitch-provided) relative order, so this is a stable partition.
const twitchBadgeTier = (key: string): number =>
  isChannelSpecificTwitchBadge(key.split('/')[0]) ? 0 : 1;

/**
 * Order a chatter's parsed Twitch IRC badges so the channel-contextual ones
 * (subscriber, predictions/poll, …) come before the global identity ones
 * (partner, prime, …). Stable — Array.prototype.sort preserves order within a
 * tier. Returns a new array; the input is not mutated.
 */
export function orderTwitchBadges<T extends { key: string }>(badges: T[]): T[] {
  return [...badges].sort((a, b) => twitchBadgeTier(a.key) - twitchBadgeTier(b.key));
}
