// Category drill: live streams for one category, layered above the tab shell
// (and below the watch layer, so back exits the stream first).
//
// A chip row under the title filters the list by Twitch's free-form tags (every
// picked tag must match; Twitch filters server-side). The picks live in this
// screen's state: it stays mounted under the watch layer, so watching a stream
// and coming back keeps them, and leaving the category unmounts it, so they go.
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { CaretLeft } from 'phosphor-react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import { useMobileNavStore } from '../navStore';
import { DrillInScreen } from '../ui/DrillInScreen';
import { useHypeTrains } from '../useHypeTrains';
import { MobileStreamCard } from '../ui/MobileStreamCard';
import { AdaptiveGrid } from '../ui/AdaptiveGrid';
import { SettleIn, useSettleIn } from '../ui/SettleIn';
import { PullToRefresh } from '../ui/PullToRefresh';
import { SkeletonCards } from '../ui/SkeletonCards';
import { EmptyNote } from '../ui/EmptyNote';
import { CategoryTagRail } from '../ui/CategoryTagRail';
import { CategoryTagSheet } from '../ui/CategoryTagSheet';
import { useCategoryTagSuggestions } from '../../hooks/useCategoryTagSuggestions';
import { Logger } from '../../utils/logger';
import {
  applyPage,
  beginMore,
  failPage,
  newTagList,
  NO_TAGS,
  refreshTagList,
  toggleTag,
  type TagList,
} from '../../utils/tagFilter';
import type { TwitchStream } from '../../types';
import { bumpPreviewStamp } from '../followRefresh';

type Category = NonNullable<ReturnType<typeof useMobileNavStore.getState>['browseCategory']>;

// Streams per request. Matches the categories grid; big enough that scrolling
// rarely waits, small enough that the first paint is quick on mobile data.
const PAGE = 40;
/** Ceiling on how much of a category the infinite scroll will accumulate. */
const MAX_LOADED_STREAMS = 320;
/** The chip row asks for everything Rust would add; no row comes near it. */
const ROW_LIMIT = 200;

export const CategoryStreamsScreen: React.FC = () => {
  const category = useMobileNavStore((s) => s.browseCategory);
  // Retained so the list still renders while the layer slides OUT; `category`
  // is already null by then and the screen would blank mid-animation.
  const [last, setLast] = useState<Category | null>(category);
  if (category && category !== last) setLast(category);

  return (
    <DrillInScreen
      open={!!category}
      layerKey="category"
      className="absolute inset-0 z-[35] bg-background flex flex-col"
      style={{ paddingTop: 'var(--sn-safe-t, 0px)' }}
    >
      {/* Keyed on the category, so opening a different one MOUNTS A FRESH
          component with an empty list already loading (and no picks). The
          alternative - clearing state in an effect - paints the previous
          category's streams under the new title for a frame first, and costs
          an extra render every time. */}
      {last && <CategoryStreamsList key={last.id} category={last} />}
    </DrillInScreen>
  );
};

const CategoryStreamsList: React.FC<{ category: Category }> = ({ category }) => {
  const openBrowseCategory = useMobileNavStore((s) => s.openBrowseCategory);
  const startStream = useAppStore((s) => s.startStream);
  const activeHypeTrainChannels = useAppStore((s) => s.activeHypeTrainChannels);
  const watchStreaks = useAppStore((s) => s.watchStreaks);

  const [streams, setStreams] = useState<TwitchStream[]>([]);
  const [loading, setLoading] = useState(true);
  const [cursor, setCursor] = useState<string | null>(null);
  const [hasMore, setHasMore] = useState(true);
  const [loadingMore, setLoadingMore] = useState(false);
  const moreSentinel = useRef<HTMLDivElement | null>(null);
  const scroller = useRef<HTMLDivElement | null>(null);

  // The tag filter. `selectedRef` is the picks' one writer's copy, so two quick
  // taps in one frame each toggle from the other's result.
  const [selected, setSelected] = useState<string[]>([]);
  const selectedRef = useRef<string[]>([]);
  const [tagList, setTagList] = useState<TagList>(() => newTagList(0, false));
  const tagGen = useRef(0);
  // Every landed page and every refresh: Rust may know new tags for the row.
  const [pages, setPages] = useState(0);
  const [sheetOpen, setSheetOpen] = useState(false);
  const tagMode = selected.length > 0;
  const listed = tagMode ? tagList.streams : streams;

  // This screen fetched no statuses and passed no badge, so a category drill
  // showed no hype trains at all.
  useHypeTrains(listed);

  // One identity: the component is already keyed by category, so opening a
  // different one remounts and settles again on its own.
  const settled = useSettleIn(!loading && streams.length > 0);

  const load = useCallback(async () => {
    try {
      const res = await invoke<[TwitchStream[], string | null]>('get_streams_by_game', {
        gameId: category.id,
        cursor: null,
        limit: PAGE,
      });
      setStreams(res[0] ?? []);
      setCursor(res[1]);
      setHasMore(!!res[1]);
      setPages((n) => n + 1);
    } catch (err) {
      Logger.warn('[CategoryStreams] load failed:', err);
      setStreams([]);
      setHasMore(false);
    } finally {
      setLoading(false);
    }
  }, [category]);

  // One tag-filtered page for generation `gen`; a page for older picks is
  // dropped by the reducer.
  const fetchTagPage = useCallback(
    async (gen: number, tags: string[], after: string | null, append: boolean) => {
      try {
        const res = await invoke<[TwitchStream[], string | null]>('get_streams_by_game_with_tags', {
          gameName: category.name,
          tags,
          cursor: after,
          limit: PAGE,
        });
        setTagList((list) => applyPage(list, gen, res[0] ?? [], res[1], append, MAX_LOADED_STREAMS));
        setPages((n) => n + 1);
      } catch (err) {
        Logger.warn('[CategoryStreams] tag page failed:', err);
        setTagList((list) => failPage(list, gen, append));
      }
    },
    [category],
  );

  // A change of picks starts a new list at the top. Driven from the tap, never
  // from an effect, so opening the screen neither scrolls nor refetches.
  const toggle = useCallback(
    (tag: string) => {
      const next = toggleTag(selectedRef.current, tag);
      selectedRef.current = next;
      setSelected(next);
      const gen = ++tagGen.current;
      setTagList(newTagList(gen, next.length > 0));
      scroller.current?.scrollTo({ top: 0 });
      if (next.length > 0) void fetchTagPage(gen, next, null, false);
    },
    [fetchTagPage],
  );
  const tagFilter = useMemo(() => ({ selected, onToggle: toggle }), [selected, toggle]);

  const row = useCategoryTagSuggestions({
    gameName: category.name,
    query: '',
    selected,
    limit: ROW_LIMIT,
    enabled: true,
    revision: pages,
    row: true,
  });

  // A busy category runs to hundreds of live channels. The cursor for them was
  // already coming back and being thrown away, so the list stopped at the first
  // page with no sign there was anything past it.
  const loadMore = useCallback(async () => {
    // Stop rather than trim, so nothing already on screen moves under a finger
    // mid-scroll. This grid is unvirtualized and stays mounted behind the player
    // once you tap into a stream, so an unbounded scroll leaves every card, its
    // thumbnail and its blur layers resident for the rest of the session. Eight
    // pages of a category is already far more than anyone scrolls.
    if (tagMode) {
      if (tagList.loading || tagList.loadingMore || !tagList.hasMore || !tagList.cursor) return;
      const gen = tagList.gen;
      setTagList((list) => beginMore(list, gen));
      await fetchTagPage(gen, selectedRef.current, tagList.cursor, true);
      return;
    }
    if (streams.length >= MAX_LOADED_STREAMS) return;
    if (loadingMore || !hasMore || !cursor) return;
    setLoadingMore(true);
    try {
      const res = await invoke<[TwitchStream[], string | null]>('get_streams_by_game', {
        gameId: category.id,
        cursor,
        limit: PAGE,
      });
      const page = res[0] ?? [];
      setStreams((prev) => {
        // Live viewer counts reorder underneath a cursor, so the same channel
        // can arrive on two pages and a duplicate key would break the grid.
        const seen = new Set(prev.map((s) => s.id));
        return [...prev, ...page.filter((s) => !seen.has(s.id))].slice(0, MAX_LOADED_STREAMS);
      });
      setCursor(res[1]);
      setHasMore(!!res[1] && page.length > 0);
      setPages((n) => n + 1);
    } catch (err) {
      Logger.warn('[CategoryStreams] page failed:', err);
      setHasMore(false);
    } finally {
      setLoadingMore(false);
    }
  }, [category, cursor, hasMore, loadingMore, streams.length, tagMode, tagList, fetchTagPage]);

  // A pull asks for the current state of everything, previews included. In tag
  // mode the rows stay on screen until the new first page replaces them.
  const refresh = useCallback(() => {
    bumpPreviewStamp();
    setPages((n) => n + 1);
    if (selectedRef.current.length === 0) return load();
    const gen = ++tagGen.current;
    setTagList((list) => refreshTagList(list, gen));
    return fetchTagPage(gen, selectedRef.current, null, false);
  }, [load, fetchTagPage]);

  useEffect(() => {
    void load();
  }, [load]);

  // Observer rather than a scroll handler: PullToRefresh owns native
  // non-passive touch listeners on this same scroller, and keeping the two
  // apart avoids any chance of them interfering.
  useEffect(() => {
    const el = moreSentinel.current;
    if (!el) return;
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) void loadMore();
      },
      { rootMargin: '600px' },
    );
    io.observe(el);
    return () => io.disconnect();
  }, [loadMore]);

  const firstLoading = tagMode ? tagList.loading : loading;
  const empty = tagMode
    ? selected.length === 1
      ? 'No live streams match that tag.'
      : 'No live streams match those tags.'
    : 'No live streams in this category.';

  // The full-screen frame (position, z, background, top inset) belongs to the
  // DrillInScreen wrapper now, so this is just the contents.
  return (
    <>
      <div className="flex items-center gap-1 px-2 py-2 shrink-0">
        <button
          onClick={() => openBrowseCategory(null)}
          className="sn-touch flex items-center justify-center text-textSecondary"
          aria-label="Back to Browse"
        >
          <CaretLeft size={22} />
        </button>
        <h1 className="text-lg font-bold text-textPrimary truncate">{category.name}</h1>
      </div>
      {/* Fixed under the title, so the filter in use never scrolls away. */}
      <CategoryTagRail
        labels={row?.labels ?? NO_TAGS}
        selected={selected}
        onToggle={toggle}
        onSearch={() => setSheetOpen(true)}
      />
      <PullToRefresh onRefresh={refresh} scrollerRef={scroller}>
        {firstLoading ? (
          <SkeletonCards />
        ) : listed.length === 0 ? (
          <EmptyNote text={empty} hint={null} />
        ) : (
          // Was `grid-cols-1 md:grid-cols-2 xl:grid-cols-3`, which is the exact
          // anti-pattern AdaptiveGrid was written for: Tailwind's breakpoints
          // are viewport-width buckets, so on an unfolded Fold `md:grid-cols-2`
          // puts its gutter at 50% of the viewport - straight down the crease.
          // AdaptiveGrid aligns the gutter TO the hinge instead, and this screen
          // now matches Following and Browse rather than having its own rules.
          <>
            <AdaptiveGrid variant="card" gap={12} className="px-4">
              {listed.map((s, i) => (
                <SettleIn key={s.id} index={i} settled={settled}>
                  <MobileStreamCard
                    stream={s}
                    // Neither of these was passed, so a card here was missing
                    // identity the same card carries on Following and Browse.
                    hypeTrain={activeHypeTrainChannels.get(s.user_id) ?? undefined}
                    watchStreak={watchStreaks[s.user_id]}
                    onPress={(stream) => void startStream(stream.user_login, stream)}
                    // The screen's title already names the category.
                    showCategory={false}
                    tagFilter={tagFilter}
                  />
                </SettleIn>
              ))}
            </AdaptiveGrid>
            {/* Below the grid, so seeing it means the list is running out. The
                tab-bar clearance rides here to stay the last thing on the page. */}
            <div ref={moreSentinel} className="sn-tabbar-clearance">
              {(tagMode ? tagList.loadingMore : loadingMore) && (
                <div className="py-4 text-center text-[12px] text-textMuted">
                  Loading more streams
                </div>
              )}
            </div>
          </>
        )}
      </PullToRefresh>
      <CategoryTagSheet
        open={sheetOpen}
        onClose={() => setSheetOpen(false)}
        gameName={category.name}
        selected={selected}
        onToggle={toggle}
        revision={pages}
      />
    </>
  );
};
