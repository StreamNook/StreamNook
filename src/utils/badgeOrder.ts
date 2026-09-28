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
//   1-2. Twitch badges               - in Twitch's own order, which Rust sets
//                                       before a row reaches the page
//                                       (chat_layout::twitch_badge_rank): role
//                                       (broadcaster, mod, VIP), subscription,
//                                       earned in the channel, then global.
//   3. 7TV badge
//   4. Third-party badges            - BTTV / FFZ / Chatterino / Homies / BTTV Pro
//                                       (any order among themselves)
//   5. StreamNook member badge       - who you are on StreamNook. Rightmost, next
//                                       to the name, where readers look first.
//
// Profiles follow the same order: a member card's worn-badge row, and the user
// card's badge panel, which groups badges by provider in that sequence.
// Chat surfaces order the tiers by laying their JSX blocks out in this sequence.
// This module keeps the lists of channel-scoped Twitch badge sets.

// Twitch badge SET ids that are scoped to the current channel (different image /
// meaning per channel) rather than global identity. Drives image caching:
// channel-scoped badges are never cached, so one channel's sub badge never
// bleeds into another.
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
 * A superset of CHANNEL_SPECIFIC_TWITCH_BADGES, which only drives caching.
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

