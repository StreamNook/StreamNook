// Hype-train badges for whatever stream list a screen is showing.
//
// The port's recurring failure mode, and this is another instance of it: mobile
// RENDERS a signal that the desktop component also FETCHES, and only the fetch
// is missing. `BrowseScreen` already read `activeHypeTrainChannels` and drew the
// badge, but nothing ever populated it for browse results, so a train only
// appeared if that channel happened to ALSO be in your following list.
// `CategoryStreamsScreen` did not even pass the prop. Following worked purely
// because `FollowingScreen` does its own fetch.
//
// Desktop has one owner for this: `Home.tsx` hands the Rust-owned Home
// snapshot (services::home_snapshot) the ids it has on screen beyond the
// followed list via `set_home_extra_channels`, debounced 2s so the request does
// not compete with HLS segments, and the snapshot's hype-train poll covers them
// from then on. The mobile shell has no equivalent single place -- every screen
// owns its own list -- so this hook is that owner, per screen. Statuses arrive
// as `home-snapshot` events the store applies to `activeHypeTrainChannels`.
// The snapshot's collaborations poll reads the same ids, so Shared Viewership
// reaches these cards through this hook too.
import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { TwitchStream } from '../types';
import { isTwitchStream } from '../utils/streamProvider';

// Matches desktop. The delay is not politeness, it is to keep this off the wire
// while HLS segments are in flight.
const DEBOUNCE_MS = 2000;

export function useHypeTrains(streams: TwitchStream[]): void {
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Key on the SET of ids, not the array. A list re-fetched with the same
  // channels (pull-to-refresh, a viewer-count tick) produces a new array every
  // time, and depending on that would re-request on every render.
  const key = Array.from(new Set(streams.filter(isTwitchStream).map((s) => s.user_id).filter(Boolean)))
    .sort()
    .join(',');

  useEffect(() => {
    if (!key) return;
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => {
      void invoke('set_home_extra_channels', { channelIds: key.split(',') }).catch(() => {});
    }, DEBOUNCE_MS);
    return () => {
      if (timer.current) clearTimeout(timer.current);
    };
  }, [key]);
}
