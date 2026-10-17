// The phone's Following list: every live follow across platforms, merged,
// deduped and ranked by viewers in Rust (`unified_following`, the home
// snapshot's `following.followed_live`). The phone keeps the platforms it can
// play; Twitch `xqc` and Kick `xqc` stay two rows in their own key spaces.
import type { FollowingLists, TwitchStream } from '../types';
import { streamProvider } from '../utils/streamProvider';

/** Platforms the phone can watch today. YouTube and TikTok rows stay out. */
const PHONE_PROVIDERS: ReadonlySet<string> = new Set(['twitch', 'kick']);

export function phoneFollowedLive(following: FollowingLists): TwitchStream[] {
  return (following.followed_live ?? []).filter((r) => PHONE_PROVIDERS.has(streamProvider(r)));
}

/** Whether you follow a Kick channel (the platform filter's gate). */
export function followsKick(following: FollowingLists): boolean {
  return (following.platforms ?? []).includes('kick');
}
