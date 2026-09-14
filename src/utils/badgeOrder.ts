// Single source of truth for the order badges render in on chat ROWS (in-app
// chat and the overlay renderer). The canonical order, left to right (badges sit
// before the username, so "last" = rightmost = adjacent to the name; YouTube rows
// put badges AFTER the name, where "last" is still the rightmost slot):
//
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
// The profile card is exempt: it groups badges by PROVIDER in labeled sections
// and deliberately leads with StreamNook (see UserProfileCard's badges panel).
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
