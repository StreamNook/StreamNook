// Twitch and Kick channels always have subscriber art (Rust's
// `channel_sub_badge` answers the channel's own badge, else the platform's
// standard one), so while it loads their Subscribe control keeps an empty slot
// instead of drawing a placeholder. YouTube keeps its own icon when a channel
// has no member badge.
export function subscribeBadgePending(provider: string, url: string | null | undefined): boolean {
  return !url && (provider === 'twitch' || provider === 'kick');
}
