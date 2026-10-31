// The phone's Following lists: live favorites on their shelf, then every other
// live follow, both merged, deduped and ranked by viewers in Rust
// (`unified_following`, the home snapshot's `following.favorites` and
// `following.live`; Rust already leaves the favorites out of `live`). The phone
// keeps the platforms it can play and the platform filter; Twitch `xqc` and
// Kick `xqc` stay two rows in their own key spaces.
import type { FollowingLists, TwitchStream } from '../types';
import { streamProvider } from '../utils/streamProvider';

/** Platforms the phone can watch today. YouTube and TikTok rows stay out. */
const PHONE_PROVIDERS: ReadonlySet<string> = new Set(['twitch', 'kick']);

export interface FollowingSections {
  /** Live favorites, most watched first. */
  favorites: TwitchStream[];
  /** Every other live follow, most watched first. */
  rest: TwitchStream[];
  /** The two parts together hold more than one platform (rows wear marks). */
  mixed: boolean;
}

export function followingSections(following: FollowingLists, filter: 'twitch' | 'kick' | null): FollowingSections {
  const keep = (rows: TwitchStream[] | undefined) =>
    (rows ?? []).filter((r) => {
      const provider = streamProvider(r);
      return PHONE_PROVIDERS.has(provider) && (filter === null || provider === filter);
    });
  const favorites = keep(following.favorites);
  const rest = keep(following.live);
  const mixed = new Set([...favorites, ...rest].map((r) => streamProvider(r))).size > 1;
  return { favorites, rest, mixed };
}

/** Whether you follow a Kick channel (the platform filter's gate). */
export function followsKick(following: FollowingLists): boolean {
  return (following.platforms ?? []).includes('kick');
}
