// Followed live channels: card feed or compact list, user's choice persisted.
import React, { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { SettleIn, useSettleIn } from '../ui/SettleIn';
import { useAppStore } from '../../stores/AppStore';
import { bumpPreviewStamp } from '../followRefresh';
import { MobileStreamCard } from '../ui/MobileStreamCard';
import { PullToRefresh } from '../ui/PullToRefresh';
import { SkeletonCards } from '../ui/SkeletonCards';
import { AdaptiveGrid } from '../ui/AdaptiveGrid';
import type { TwitchStream } from '../../types';
import { isTwitchStream, streamKey, streamProvider } from '../../utils/streamProvider';
import { followsKick, phoneFollowedLive } from '../followingMerge';
import { PROVIDERS } from '../../types/providers';
import { ALL_PLATFORMS, KICK_PLATFORM, PlatformMenu, TWITCH_PLATFORM } from '../ui/PlatformMenu';
import { EmptyNote } from '../ui/EmptyNote';
import { ScreenHeader } from '../ui/ScreenHeader';
import { TitleCapsule, ViewToggle } from '../ui/TitleControls';

/** Which platform Following shows. null is everything. */
type PlatformFilter = 'twitch' | 'kick' | null;
const FILTER_KEY = 'sn-following-platform';
function readFilter(): PlatformFilter {
  try {
    const v = localStorage.getItem(FILTER_KEY);
    return v === 'twitch' || v === 'kick' ? v : null;
  } catch {
    return null;
  }
}
function writeFilter(v: PlatformFilter): void {
  try {
    if (v) localStorage.setItem(FILTER_KEY, v);
    else localStorage.removeItem(FILTER_KEY);
  } catch {
    /* a remembered filter is a convenience; losing it shows everything */
  }
}

export type StreamViewMode = 'cards' | 'list';
// One shared view preference for every stream list (Following + Browse).
const VIEW_KEY = 'sn-stream-view';
const LEGACY_VIEW_KEY = 'sn-following-view';

export function readStreamView(): StreamViewMode {
  const v = localStorage.getItem(VIEW_KEY) ?? localStorage.getItem(LEGACY_VIEW_KEY);
  return v === 'list' ? 'list' : 'cards';
}

export function writeStreamView(mode: StreamViewMode): void {
  localStorage.setItem(VIEW_KEY, mode);
}

export const FollowingScreen: React.FC = () => {
  const followedStreams = useAppStore((s) => s.followedStreams);
  // Every live follow across platforms, merged and ranked in Rust.
  const following = useAppStore((s) => s.following);
  const merged = useMemo(() => phoneFollowedLive(following), [following]);
  // The filter only exists once there is a second platform to filter by.
  const hasKick = followsKick(following);
  const [filter, setFilter] = useState<PlatformFilter>(readFilter);
  const activeFilter = hasKick ? filter : null;
  const rows = useMemo(
    () => (activeFilter ? merged.filter((s) => streamProvider(s) === activeFilter) : merged),
    [merged, activeFilter],
  );
  // The menu's choice: everything, or one platform.
  const chooseFilter = (choice: 'all' | 'twitch' | 'kick') => {
    const next = choice === 'all' ? null : choice;
    setFilter(next);
    writeFilter(next);
  };
  // Kick rows wear Kick's mark only where the list mixes platforms; a list of
  // one platform (filtered, or simply all Twitch right now) needs no marks.
  const mixed = useMemo(() => new Set(rows.map((s) => streamProvider(s))).size > 1, [rows]);
  const loadFollowedStreams = useAppStore((s) => s.loadFollowedStreams);
  const startStream = useAppStore((s) => s.startStream);
  const activeHypeTrainChannels = useAppStore((s) => s.activeHypeTrainChannels);
  const watchStreaks = useAppStore((s) => s.watchStreaks);
  const [firstLoad, setFirstLoad] = useState(followedStreams.length === 0);
  // View choice persists: if a user can choose it, it survives restart.
  const [view, setView] = useState<StreamViewMode>(readStreamView);

  // Keyed on the view, so switching between cards and list re-settles. Every
  // row changes shape and size in that swap, which is a big enough visual
  // change to deserve being animated rather than snapping; a refresh, where the
  // same cards stay the same shape, still does not replay.
  const settled = useSettleIn(!firstLoad && rows.length > 0, `${view}:${activeFilter ?? 'all'}`);

  const setViewPersisted = (mode: StreamViewMode) => {
    setView(mode);
    writeStreamView(mode);
  };

  useEffect(() => {
    const boot = async () => {
      if (useAppStore.getState().followedStreams.length === 0) {
        await loadFollowedStreams().catch(() => {});
        bumpPreviewStamp();
        setFirstLoad(false);
      }
      // Hype train badges are a section of the Rust-owned Home snapshot; the
      // followed list is polled for them on its own cadence, this just asks for
      // a fresh pass now that the list is on screen (floored at 15 s in Rust).
      void invoke('refresh_home_section', { section: 'hype_trains' }).catch(() => {});
    };
    void boot();
    // Loads once on mount. Staying fresh after that is the resume handler's job
    // (lifecycle.ts -> refreshFollowingIfStale), because this effect is guarded
    // on the list being EMPTY and the Android activity survives backgrounding,
    // so nothing here re-runs when the user comes back to a days-old list.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const refresh = async () => {
    // One Rust call: Twitch's followed list (stamping the freshness the resume
    // path shares), a live re-check of Kick and the other platforms you follow,
    // and hype trains. The lists arrive as home-snapshot pushes.
    await invoke('refresh_following').catch(() => {});
    bumpPreviewStamp();
  };

  const onPress = (stream: TwitchStream) => {
    void startStream(stream.user_login, stream);
  };

  return (
    <div className="flex-1 min-h-0 flex flex-col">
      {/* The filter only exists once you follow someone on Kick, and sits in
          the title row so it spends no row of its own. */}
      <ScreenHeader title="Following">
        {hasKick && (
          <TitleCapsule>
            <PlatformMenu
              options={[ALL_PLATFORMS, TWITCH_PLATFORM, KICK_PLATFORM]}
              value={activeFilter ?? 'all'}
              onChange={chooseFilter}
            />
          </TitleCapsule>
        )}
        <ViewToggle value={view} onChange={setViewPersisted} />
      </ScreenHeader>
      <PullToRefresh
        onRefresh={refresh}
        className="px-0 [padding-left:var(--sn-safe-l)] [padding-right:var(--sn-safe-r)]"
      >
        {firstLoad ? (
          <SkeletonCards />
        ) : rows.length === 0 ? (
          <EmptyNote
            text={
              activeFilter
                ? `No ${PROVIDERS[activeFilter].label} channels you follow are live right now.`
                : 'No followed channels are live right now.'
            }
          />
        ) : (
          <AdaptiveGrid
            variant={view === 'list' ? 'row' : 'card'}
            gap={view === 'list' ? 8 : 12}
            className="px-4 sn-tabbar-clearance"
          >
            {rows.map((s, i) => (
              <SettleIn key={streamKey(s)} index={i} settled={settled}>
                <MobileStreamCard
                  stream={s}
                  hypeTrain={isTwitchStream(s) ? (activeHypeTrainChannels.get(s.user_id) ?? undefined) : undefined}
                  watchStreak={isTwitchStream(s) ? watchStreaks[s.user_id] : undefined}
                  onPress={onPress}
                  variant={view === 'list' ? 'row' : 'card'}
                  showPlatform={mixed}
                />
              </SettleIn>
            ))}
          </AdaptiveGrid>
        )}
      </PullToRefresh>
    </div>
  );
};
