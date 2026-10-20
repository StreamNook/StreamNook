// Discover: Live streams and Categories, with search covering both modes.
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import { useMobileNavStore } from '../navStore';
import { useHypeTrains } from '../useHypeTrains';
import { SettleIn, useSettleIn } from '../ui/SettleIn';
import { MobileStreamCard } from '../ui/MobileStreamCard';
import { OfflineChannelsGroup } from './OfflineChannelsGroup';
import { readStreamView, writeStreamView, type StreamViewMode } from './FollowingScreen';
import { PullToRefresh } from '../ui/PullToRefresh';
import { SkeletonCards, SkeletonTiles } from '../ui/SkeletonCards';
import { CategoryTile } from '../ui/CategoryTile';
import { TILE_GRID_CLASS } from '../ui/categoryTileLayout';
import { AdaptiveGrid } from '../ui/AdaptiveGrid';
import { Logger } from '../../utils/logger';
import type { TwitchCategory, TwitchStream } from '../../types';
import { bumpPreviewStamp } from '../followRefresh';
import { isTwitchStream, streamKey } from '../../utils/streamProvider';
import { KICK_PLATFORM, PlatformMenu, TWITCH_PLATFORM } from '../ui/PlatformMenu';
import { SegmentedControl, type Segment } from '../ui/SegmentedControl';
import { ScreenHeader } from '../ui/ScreenHeader';
import { TitleCapsule, ViewToggle } from '../ui/TitleControls';
import { SearchField } from '../ui/SearchField';
import { EmptyNote } from '../ui/EmptyNote';
import type { ProviderCategory } from '../../types/providers';
import { KickBrowseBody } from './KickBrowseBody';

type BrowseMode = 'live' | 'categories';
type BrowsePlatform = 'twitch' | 'kick';

const PLATFORM_KEY = 'sn-browse-platform';
function readPlatform(): BrowsePlatform {
  try {
    return localStorage.getItem(PLATFORM_KEY) === 'kick' ? 'kick' : 'twitch';
  } catch {
    return 'twitch';
  }
}
function writePlatform(p: BrowsePlatform): void {
  try {
    localStorage.setItem(PLATFORM_KEY, p);
  } catch {
    /* a remembered tab is a convenience; losing it is fine */
  }
}

/** Stable empty list, so Kick mode hands the hype-train hook nothing to look up. */
const NO_STREAMS: TwitchStream[] = [];

const MODE_SEGMENTS: Segment<BrowseMode>[] = [
  { value: 'live', label: 'Live' },
  { value: 'categories', label: 'Categories' },
];

// One page of categories. Twitch orders games only roughly by viewers, so the
// backend sums each game's viewers and sorts the page on that, which is also
// why a page costs a request per game and is worth keeping to a sensible size.
const CATEGORY_PAGE = 40;

// Top categories, module-cached for the session (pull-to-refresh reloads).
// Carries the paging cursor with the items: without it, coming back to Browse
// restored the list but not the position in it, so scrolling would have
// restarted from page two every time.
let topCategoriesCache: {
  items: TwitchCategory[];
  cursor: string | null;
  hasMore: boolean;
} | null = null;

export const BrowseScreen: React.FC = () => {
  const recommendedStreams = useAppStore((s) => s.recommendedStreams);
  const loadRecommendedStreams = useAppStore((s) => s.loadRecommendedStreams);
  const startStream = useAppStore((s) => s.startStream);
  const startOfflineChat = useAppStore((s) => s.startOfflineChat);
  const openBrowseCategory = useMobileNavStore((s) => s.openBrowseCategory);

  const [mode, setMode] = useState<BrowseMode>('live');
  const [platform, setPlatform] = useState<BrowsePlatform>(readPlatform);
  const [kickCategory, setKickCategory] = useState<ProviderCategory | null>(null);
  const switchPlatform = (p: BrowsePlatform) => {
    if (p === platform) return;
    setPlatform(p);
    writePlatform(p);
    setQuery('');
    setKickCategory(null);
  };
  const [view, setView] = useState<StreamViewMode>(readStreamView);
  const activeHypeTrainChannels = useAppStore((s) => s.activeHypeTrainChannels);
  const watchStreaks = useAppStore((s) => s.watchStreaks);

  const setViewPersisted = (v: StreamViewMode) => {
    setView(v);
    writeStreamView(v);
  };
  const [query, setQuery] = useState('');
  const [streamResults, setStreamResults] = useState<TwitchStream[] | null>(null);
  const [categoryResults, setCategoryResults] = useState<TwitchCategory[] | null>(null);
  const [categories, setCategories] = useState<TwitchCategory[]>(
    () => topCategoriesCache?.items ?? [],
  );
  const [categoriesCursor, setCategoriesCursor] = useState<string | null>(
    () => topCategoriesCache?.cursor ?? null,
  );
  const [hasMoreCategories, setHasMoreCategories] = useState(
    () => topCategoriesCache?.hasMore ?? true,
  );
  const [loadingMore, setLoadingMore] = useState(false);
  const moreSentinel = useRef<HTMLDivElement | null>(null);
  const [searching, setSearching] = useState(false);
  const [firstLoad, setFirstLoad] = useState(recommendedStreams.length === 0);
  const [categoriesLoading, setCategoriesLoading] = useState(false);
  const searchSeq = useRef(0);
  const searchInput = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (recommendedStreams.length === 0) {
      void loadRecommendedStreams().finally(() => setFirstLoad(false));
    } else {
      setFirstLoad(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const loadCategories = async () => {
    setCategoriesLoading(true);
    try {
      const [games, cursor] = await invoke<[TwitchCategory[], string | null]>(
        'get_top_games_paginated',
        { cursor: null, limit: CATEGORY_PAGE },
      );
      const items = games ?? [];
      topCategoriesCache = { items, cursor, hasMore: !!cursor };
      setCategories(items);
      setCategoriesCursor(cursor);
      setHasMoreCategories(!!cursor);
    } catch (err) {
      Logger.warn('[BrowseScreen] categories load failed:', err);
    } finally {
      setCategoriesLoading(false);
    }
  };

  // The next page. Twitch's category list runs to hundreds of entries, and the
  // cursor to reach them was already coming back from the backend and being
  // dropped on the floor, which is the whole reason this list used to stop
  // dead well short of what Twitch itself shows.
  const loadMoreCategories = async () => {
    if (loadingMore || !hasMoreCategories || !categoriesCursor) return;
    setLoadingMore(true);
    try {
      const [games, cursor] = await invoke<[TwitchCategory[], string | null]>(
        'get_top_games_paginated',
        { cursor: categoriesCursor, limit: CATEGORY_PAGE },
      );
      const page = games ?? [];
      setCategories((prev) => {
        // Twitch can repeat an entry across a cursor boundary as the live
        // viewer ordering shifts underneath, and a duplicate key would break
        // the grid.
        const seen = new Set(prev.map((c) => c.id));
        const merged = [...prev, ...page.filter((c) => !seen.has(c.id))];
        topCategoriesCache = { items: merged, cursor, hasMore: !!cursor && page.length > 0 };
        return merged;
      });
      setCategoriesCursor(cursor);
      setHasMoreCategories(!!cursor && page.length > 0);
    } catch (err) {
      Logger.warn('[BrowseScreen] categories page failed:', err);
      // Stop asking rather than retry against the sentinel forever.
      setHasMoreCategories(false);
    } finally {
      setLoadingMore(false);
    }
  };

  useEffect(() => {
    if (platform === 'twitch' && mode === 'categories' && categories.length === 0) void loadCategories();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, platform]);

  // Load the next page when the end of the list comes into view.
  //
  // An observer rather than a scroll handler on purpose: this grid lives inside
  // PullToRefresh, which owns native non-passive touch listeners on the same
  // scroller, and adding scroll-position maths beside them is asking for the
  // two to interfere. The observer never touches the event stream.
  useEffect(() => {
    const el = moreSentinel.current;
    if (!el) return;
    if (mode !== 'categories' || categoryResults) return;
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) void loadMoreCategories();
      },
      // A screen of lead time, so the next page is usually already there by the
      // time the current one runs out.
      { rootMargin: '600px' },
    );
    io.observe(el);
    return () => io.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, categoryResults, categoriesCursor, hasMoreCategories, loadingMore]);

  const runSearch = async (q: string, seq: number, searchMode: BrowseMode) => {
    try {
      if (searchMode === 'live') {
        const found = await invoke<TwitchStream[]>('search_channels', { query: q });
        if (seq === searchSeq.current) setStreamResults(found);
      } else {
        const found = await invoke<TwitchCategory[]>('search_categories', {
          query: q,
          limit: 40,
        });
        if (seq === searchSeq.current) setCategoryResults(found);
      }
    } catch (err) {
      Logger.warn('[BrowseScreen] search failed:', err);
      if (seq === searchSeq.current) {
        if (searchMode === 'live') setStreamResults([]);
        else setCategoryResults([]);
      }
    } finally {
      if (seq === searchSeq.current) setSearching(false);
    }
  };

  // Debounced search in whichever mode is active.
  useEffect(() => {
    // Kick searches inside its own body.
    if (platform !== 'twitch') {
      setSearching(false);
      return;
    }
    const trimmed = query.trim();
    if (!trimmed) {
      setStreamResults(null);
      setCategoryResults(null);
      setSearching(false);
      return;
    }
    setSearching(true);
    const seq = ++searchSeq.current;
    const t = setTimeout(() => void runSearch(trimmed, seq, mode), 350);
    return () => clearTimeout(t);
  }, [query, mode, platform]);

  const refresh = async () => {
    // A pull asks for the current state of everything, previews included.
    bumpPreviewStamp();
    const trimmed = query.trim();
    if (trimmed) {
      const seq = ++searchSeq.current;
      setSearching(true);
      await runSearch(trimmed, seq, mode);
    } else if (mode === 'live') {
      await loadRecommendedStreams();
    } else {
      await loadCategories();
    }
  };

  const shownStreams = streamResults ?? recommendedStreams;
  // A search also finds channels that are not streaming; Rust marks them
  // `is_live: false` from Twitch's streams answer. They get their own group.
  const liveStreams = useMemo(() => shownStreams.filter((s) => s.is_live !== false), [shownStreams]);
  const offlineChannels = useMemo(() => shownStreams.filter((s) => s.is_live === false), [shownStreams]);
  // Browse already drew the badge but never fetched the statuses, so a train
  // only ever showed for channels that happened to also be in your following.
  useHypeTrains(platform === 'twitch' ? shownStreams : NO_STREAMS);
  const shownCategories = categoryResults ?? categories;

  // Keyed on WHICH list is showing, not on its contents. A new search is a new
  // list and settles in; loading the next page of the same list is the same
  // list getting longer, so those rows simply appear.
  // `view` is in the key for the same reason as Following: switching between
  // cards and list reshapes every row, which is worth animating.
  const streamsSettled = useSettleIn(
    !firstLoad && shownStreams.length > 0,
    `${view}:${streamResults ? `q:${query.trim()}` : 'recommended'}`,
  );
  const categoriesSettled = useSettleIn(
    !categoriesLoading && shownCategories.length > 0,
    categoryResults ? `q:${query.trim()}` : 'top',
  );

  return (
    // A result tapped while typing a search opens, and the keyboard goes.
    <div className="flex-1 min-h-0 flex flex-col" data-taps-through-keyboard="">
      <div className="shrink-0">
        {/* The same logo menu Following filters with; switching clears the
            search and any Kick category. The view toggle joins it in the
            title row while the live list shows, so the mode row below holds
            one control and never changes height. */}
        <ScreenHeader title="Browse">
          <TitleCapsule>
            <PlatformMenu options={[TWITCH_PLATFORM, KICK_PLATFORM]} value={platform} onChange={switchPlatform} />
          </TitleCapsule>
          {mode === 'live' && <ViewToggle value={view} onChange={setViewPersisted} />}
        </ScreenHeader>
        <div className="flex flex-col gap-2.5 px-4 pb-2">
          <SearchField
            ref={searchInput}
            value={query}
            onChange={setQuery}
            placeholder={
              mode === 'live' ? (platform === 'kick' ? 'Search Kick channels' : 'Search channels') : 'Search categories'
            }
          />
          <div className="flex">
            <SegmentedControl label="Show" segments={MODE_SEGMENTS} value={mode} onChange={setMode} />
          </div>
        </div>
      </div>
      {/* Touching or scrolling the results puts the keyboard away, so it
          never sits over the list you are reading. */}
      <div className="flex-1 min-h-0 flex flex-col">
        {platform === 'kick' ? (
          <KickBrowseBody
            mode={mode}
            query={query}
            view={view}
            category={kickCategory}
            onPickCategory={(c) => {
              setKickCategory(c);
              setQuery('');
              setMode('live');
            }}
            onClearCategory={() => setKickCategory(null)}
          />
        ) : (
          <PullToRefresh onRefresh={refresh}>
            {mode === 'live' ? (
              firstLoad || (searching && shownStreams.length === 0) ? (
                <SkeletonCards />
              ) : shownStreams.length === 0 ? (
                <EmptyNote text={streamResults ? 'No channels found.' : 'Nothing to recommend yet.'} />
              ) : (
                <div className="sn-tabbar-clearance">
                  {liveStreams.length > 0 && (
                    <AdaptiveGrid
                      variant={view === 'list' ? 'row' : 'card'}
                      gap={view === 'list' ? 8 : 12}
                      className="px-4"
                    >
                      {liveStreams.map((s, i) => (
                        <SettleIn key={streamKey(s)} index={i} settled={streamsSettled}>
                          <MobileStreamCard
                            stream={s}
                            hypeTrain={isTwitchStream(s) ? (activeHypeTrainChannels.get(s.user_id) ?? undefined) : undefined}
                            watchStreak={isTwitchStream(s) ? watchStreaks[s.user_id] : undefined}
                            onPress={(stream) => void startStream(stream.user_login, stream)}
                            variant={view === 'list' ? 'row' : 'card'}
                          />
                        </SettleIn>
                      ))}
                    </AdaptiveGrid>
                  )}
                  {offlineChannels.length > 0 && (
                    <OfflineChannelsGroup
                      className={liveStreams.length > 0 ? 'mt-5' : ''}
                      channels={offlineChannels}
                      onOpen={(c) => void startOfflineChat(c.user_login, c)}
                    />
                  )}
                </div>
              )
            ) : categoriesLoading || (searching && shownCategories.length === 0) ? (
              <SkeletonTiles />
            ) : shownCategories.length === 0 ? (
              <EmptyNote
                text={categoryResults ? 'No categories found.' : 'No categories yet, pull to refresh.'}
                hint={null}
              />
            ) : (
              <>
              <div className={`${TILE_GRID_CLASS} px-4`}>
                {shownCategories.map((c, i) => (
                  <SettleIn key={c.id} index={i} settled={categoriesSettled}>
                    <CategoryTile
                      name={c.name}
                      boxArt={c.box_art_url}
                      viewers={c.viewer_count}
                      viewersText={c.viewers_line}
                      twoLineName={!!categoryResults}
                      onPress={() => openBrowseCategory(c)}
                    />
                  </SettleIn>
                ))}
              </div>
              {/* Sits below the grid so coming into view means the list is running
                  out. Only while browsing: search returns one fixed set of results
                  and has no cursor to follow. The tab-bar clearance moved here from
                  the grid so it stays the last thing on the page. */}
              {!categoryResults && (
                <div ref={moreSentinel} className="sn-tabbar-clearance">
                  {loadingMore && (
                    <div className="py-4 text-center text-[12px] text-textMuted">
                      Loading more categories
                    </div>
                  )}
                </div>
              )}
              {categoryResults && <div className="sn-tabbar-clearance" />}
              </>
            )}
          </PullToRefresh>
        )}
      </div>
    </div>
  );
};
