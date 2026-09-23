// Followed live channels: card feed or compact list, user's choice persisted.
import React, { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { SettleIn, useSettleIn } from '../ui/SettleIn';
import { ListBullets, SquaresFour } from 'phosphor-react';
import { useAppStore } from '../../stores/AppStore';
import { markFollowingFresh } from '../followRefresh';
import { MobileStreamCard } from '../ui/MobileStreamCard';
import { useDropsGameNames } from '../dropsCampaigns';
import { PullToRefresh } from '../ui/PullToRefresh';
import { SkeletonCards } from '../ui/SkeletonCards';
import { AdaptiveGrid } from '../ui/AdaptiveGrid';
import type { TwitchStream } from '../../types';
import { useFollowsStore } from '../../stores/followsStore';
import { isTwitchStream, streamKey, streamProvider } from '../../utils/streamProvider';
import { mergeFollowedLive } from '../followingMerge';
import { ProviderMark } from '../../components/ProviderLogo';
import { PROVIDERS } from '../../types/providers';

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
  // Live Kick follows, pushed by Rust's provider_live_service.
  const providerLive = useFollowsStore((s) => s.liveByKey);
  const merged = useMemo(() => mergeFollowedLive(followedStreams, providerLive), [followedStreams, providerLive]);
  // The filter only exists once there is a second platform to filter by.
  const hasKick = useFollowsStore((s) => s.follows.some((f) => f.provider === 'kick'));
  const [filter, setFilter] = useState<PlatformFilter>(readFilter);
  const activeFilter = hasKick ? filter : null;
  const rows = useMemo(
    () => (activeFilter ? merged.filter((s) => streamProvider(s) === activeFilter) : merged),
    [merged, activeFilter],
  );
  // Tap a platform to show only it; tap it again to show everything.
  const toggleFilter = (p: 'twitch' | 'kick') => {
    const next = filter === p ? null : p;
    setFilter(next);
    writeFilter(next);
  };
  const loadFollowedStreams = useAppStore((s) => s.loadFollowedStreams);
  const startStream = useAppStore((s) => s.startStream);
  const activeHypeTrainChannels = useAppStore((s) => s.activeHypeTrainChannels);
  const watchStreaks = useAppStore((s) => s.watchStreaks);
  const [firstLoad, setFirstLoad] = useState(followedStreams.length === 0);
  // View choice persists: if a user can choose it, it survives restart.
  const [view, setView] = useState<StreamViewMode>(readStreamView);
  const dropsGameNames = useDropsGameNames();

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
        markFollowingFresh();
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
    void useFollowsStore.getState().refreshLive();
    await loadFollowedStreams();
    // Shares the throttle with the resume path, so pulling to refresh and then
    // switching away and back does not fetch the same thing twice.
    markFollowingFresh();
    await invoke('refresh_home_section', { section: 'hype_trains' }).catch(() => {});
  };

  const onPress = (stream: TwitchStream) => {
    void startStream(stream.user_login, stream);
  };

  return (
    <div className="flex-1 min-h-0 flex flex-col">
      <div className="flex items-center justify-between px-4 pt-3 pb-2 shrink-0">
        <h1 className="text-xl font-bold text-textPrimary">Following</h1>
        <div className="flex">
          <button
            onClick={() => setViewPersisted('cards')}
            className={`sn-touch flex items-center justify-center ${
              view === 'cards' ? 'text-accent' : 'text-textMuted'
            }`}
            aria-label="Card view"
          >
            <SquaresFour size={20} weight={view === 'cards' ? 'fill' : 'regular'} />
          </button>
          <button
            onClick={() => setViewPersisted('list')}
            className={`sn-touch flex items-center justify-center ${
              view === 'list' ? 'text-accent' : 'text-textMuted'
            }`}
            aria-label="List view"
          >
            <ListBullets size={20} weight={view === 'list' ? 'bold' : 'regular'} />
          </button>
        </div>
      </div>
      {hasKick && (
        <div className="flex items-center gap-1.5 px-4 pb-2 shrink-0">
          {(['twitch', 'kick'] as const).map((p) => {
            const on = activeFilter === p;
            return (
              <button
                key={p}
                onClick={() => toggleFilter(p)}
                aria-pressed={on}
                className={`flex items-center gap-1.5 pl-2.5 pr-3.5 py-1.5 rounded-full text-sm transition-colors ${
                  on
                    ? 'chrome-glaze chrome-glaze--flat chrome-glaze--control text-textPrimary font-semibold'
                    : 'text-textMuted'
                }`}
              >
                <ProviderMark provider={p} size={15} />
                {PROVIDERS[p].label}
              </button>
            );
          })}
        </div>
      )}
      <PullToRefresh
        onRefresh={refresh}
        className="px-0 [padding-left:var(--sn-safe-l)] [padding-right:var(--sn-safe-r)]"
      >
        {firstLoad ? (
          <SkeletonCards />
        ) : rows.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-20 gap-1">
            <div className="text-sm text-textMuted">
              {activeFilter
                ? `No ${PROVIDERS[activeFilter].label} channels you follow are live right now.`
                : 'No followed channels are live right now.'}
            </div>
            <div className="text-[13px] text-textMuted">Pull down to refresh.</div>
          </div>
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
                  dropsGameNames={dropsGameNames}
                  hypeTrain={isTwitchStream(s) ? (activeHypeTrainChannels.get(s.user_id) ?? undefined) : undefined}
                  watchStreak={isTwitchStream(s) ? watchStreaks[s.user_id] : undefined}
                  onPress={onPress}
                  variant={view === 'list' ? 'row' : 'card'}
                  // Marks tell platforms apart; a filtered list is one platform.
                  showPlatform={!activeFilter}
                />
              </SettleIn>
            ))}
          </AdaptiveGrid>
        )}
      </PullToRefresh>
    </div>
  );
};
