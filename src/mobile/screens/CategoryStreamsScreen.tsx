// Category drill: live streams for one category, layered above the tab shell
// (and below the watch layer, so back exits the stream first).
//
// A chip row under the title filters the list by Twitch's free-form tags (every
// picked tag must match; Twitch filters server-side). The picks live in this
// screen's state: it stays mounted under the watch layer, so watching a stream
// and coming back keeps them, and leaving the category unmounts it, so they go.
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { CaretLeft, ShareNetwork } from 'phosphor-react';
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
import { Tooltip } from '../../components/ui/Tooltip';
import { copyCategoryShareLink } from '../../utils/shareLink';
import { getLinkedCategory, resolveLinkedCategory, useLinkedCategory } from '../../utils/linkedCategory';
import { shareText } from '../nativeBridge';

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
  const fillBrowseCategory = useMobileNavStore((s) => s.fillBrowseCategory);

  const [streams, setStreams] = useState<TwitchStream[]>([]);
  const [loading, setLoading] = useState(true);
  const [cursor, setCursor] = useState<string | null>(null);
  const [hasMore, setHasMore] = useState(true);
  const [loadingMore, setLoadingMore] = useState(false);
  // The last plain load failed (offline, Twitch refused): the empty state says
  // so instead of claiming nobody is live.
  const [loadFailed, setLoadFailed] = useState(false);
  // Bumped by the header's Try again, so the streams load runs once more.
  const [reloadKey, setReloadKey] = useState(0);
  // Every plain load takes a number and only the newest writes, so a late
  // answer from an older load (a failure landing after a Try again worked)
  // never replaces the newer list.
  const loadSeq = useRef(0);
  // The cursor the next page continues from, the load it belongs to, and
  // whether there is more. loadMore checks it right before it asks, so a page
  // from before a refresh is never asked for or appended.
  const pageRef = useRef<{ seq: number; cursor: string | null; hasMore: boolean }>({
    seq: 0,
    cursor: null,
    hasMore: true,
  });
  // The list and the name as last rendered, for the callbacks below.
  const streamsRef = useRef<TwitchStream[]>([]);
  const nameRef = useRef(category.name);
  useEffect(() => {
    streamsRef.current = streams;
  }, [streams]);
  useEffect(() => {
    nameRef.current = category.name;
  }, [category.name]);
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
  const tagMode = selected.length > 0;
  const listed = tagMode ? tagList.streams : streams;

  // This screen fetched no statuses and passed no badge, so a category drill
  // showed no hype trains at all.
  useHypeTrains(listed);

  // One identity: the component is already keyed by category, so opening a
  // different one remounts and settles again on its own.
  const settled = useSettleIn(!loading && streams.length > 0);

  // Reads only the id, so filling a link's nameless copy with its name does not
  // fetch the streams again.
  const load = useCallback(async () => {
    const seq = ++loadSeq.current;
    const newest = () => loadSeq.current === seq;
    setLoadFailed(false);
    // A retry on an empty list shows the skeleton again, not the old empty note.
    if (streamsRef.current.length === 0) setLoading(true);
    try {
      const res = await invoke<[TwitchStream[], string | null]>('get_streams_by_game', {
        gameId: category.id,
        cursor: null,
        limit: PAGE,
      });
      if (!newest()) return;
      setStreams(res[0] ?? []);
      // A link whose name has not come back still names itself from its own
      // streams, so the tag chips on the cards work without waiting for it.
      if (!nameRef.current) {
        const named = (res[0] ?? []).find((s) => s.game_id === category.id && s.game_name);
        if (named) fillBrowseCategory(category.id, { id: category.id, name: named.game_name, box_art_url: '' });
      }
      setCursor(res[1]);
      setHasMore(!!res[1]);
      pageRef.current = { seq, cursor: res[1], hasMore: !!res[1] };
      setPages((n) => n + 1);
    } catch (err) {
      if (!newest()) return;
      Logger.warn('[CategoryStreams] load failed:', err);
      setStreams([]);
      setHasMore(false);
      pageRef.current = { seq, cursor: null, hasMore: false };
      setLoadFailed(true);
    } finally {
      if (newest()) setLoading(false);
    }
  }, [category.id, fillBrowseCategory]);

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
      // Nameless (a link still asking for its name), Twitch's tag filter has no
      // category to search. The chips stay on the cards, so nothing shifts.
      if (!nameRef.current) return;
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
    // The page must continue the newest load: a refresh that started since
    // makes this cursor stale, and its answer would append an old page.
    const page = pageRef.current;
    if (page.seq !== loadSeq.current || !page.hasMore || !page.cursor) return;
    const seq = page.seq;
    setLoadingMore(true);
    try {
      const res = await invoke<[TwitchStream[], string | null]>('get_streams_by_game', {
        gameId: category.id,
        cursor: page.cursor,
        limit: PAGE,
      });
      if (loadSeq.current !== seq) return;
      const rows = res[0] ?? [];
      setStreams((prev) => {
        // Live viewer counts reorder underneath a cursor, so the same channel
        // can arrive on two pages and a duplicate key would break the grid.
        const seen = new Set(prev.map((s) => s.id));
        return [...prev, ...rows.filter((s) => !seen.has(s.id))].slice(0, MAX_LOADED_STREAMS);
      });
      const more = !!res[1] && rows.length > 0;
      setCursor(res[1]);
      setHasMore(more);
      pageRef.current = { seq, cursor: res[1], hasMore: more };
      setPages((n) => n + 1);
    } catch (err) {
      if (loadSeq.current !== seq) return;
      Logger.warn('[CategoryStreams] page failed:', err);
      setHasMore(false);
      pageRef.current = { seq, cursor: page.cursor, hasMore: false };
    } finally {
      setLoadingMore(false);
    }
  }, [category, cursor, hasMore, loadingMore, streams.length, tagMode, tagList, fetchTagPage]);

  // A pull asks for the current state of everything, previews included. In tag
  // mode the rows stay on screen until the new first page replaces them.
  const refresh = useCallback(() => {
    bumpPreviewStamp();
    setPages((n) => n + 1);
    // A pull is a fresh try at a name that failed to load, too.
    if (category.name === '' && getLinkedCategory(category.id)?.state === 'failed') {
      void resolveLinkedCategory(category.id, { retry: true });
    }
    if (selectedRef.current.length === 0) return load();
    const gen = ++tagGen.current;
    setTagList((list) => refreshTagList(list, gen));
    return fetchTagPage(gen, selectedRef.current, null, false);
  }, [load, fetchTagPage, category.id, category.name]);

  useEffect(() => {
    void load();
  }, [load, reloadKey]);

  // Another link to this category is a fresh try at every part that failed,
  // the streams included, not only the name. Compared with the value seen, so
  // the first run and a StrictMode second setup do nothing.
  const linkSheetSeq = useMobileNavStore((s) => s.linkSheetSeq);
  const seenLinkSeq = useRef(linkSheetSeq);
  const loadFailedRef = useRef(loadFailed);
  useEffect(() => {
    loadFailedRef.current = loadFailed;
  }, [loadFailed]);
  useEffect(() => {
    if (linkSheetSeq === seenLinkSeq.current) return;
    seenLinkSeq.current = linkSheetSeq;
    if (loadFailedRef.current) setReloadKey((n) => n + 1);
  }, [linkSheetSeq]);

  // A link opens this screen with the id alone; the screen asks for its name
  // (one ask per id, shared with any other screen showing it) and fills it in
  // place. A found record fills with no call. Keyed on the record's state, so a
  // new link that dropped a failed record asks again, and a late answer fills.
  const record = useLinkedCategory(category.id);
  const recordState = record?.state ?? 'none';
  useEffect(() => {
    if (category.name !== '') return;
    void resolveLinkedCategory(category.id).then((rec) => {
      if (rec.state === 'found' && rec.category) fillBrowseCategory(category.id, rec.category);
    });
  }, [category.id, category.name, recordState, fillBrowseCategory]);

  const nameless = category.name === '';
  // Asks for the name again and runs the streams load once more.
  const retryName = () => {
    void resolveLinkedCategory(category.id, { retry: true });
    setReloadKey((n) => n + 1);
  };
  // The system share sheet is the feedback; a refused link or no share sheet
  // ends quietly in copyCategoryShareLink's one catch.
  const share = () => {
    void copyCategoryShareLink(category, (link) =>
      shareText(`${category.name} on StreamNook: ${link}`, category.name) ? Promise.resolve() : Promise.reject(new Error('no share')),
    );
  };

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
    : loadFailed
      ? "Couldn't load streams"
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
        {!nameless ? (
          <h1 className="min-w-0 flex-1 truncate text-lg font-bold text-textPrimary">{category.name}</h1>
        ) : recordState === 'missing' ? (
          <p className="min-w-0 flex-1 truncate text-sm text-textSecondary">There's no Twitch category with this id.</p>
        ) : recordState === 'failed' ? (
          <>
            <p className="min-w-0 flex-1 truncate text-sm text-textSecondary">Couldn't load this category</p>
            <button
              type="button"
              onClick={retryName}
              className="sn-touch ml-auto shrink-0 px-2 text-sm font-semibold text-accent"
            >
              Try again
            </button>
          </>
        ) : (
          // The title's height, so nothing moves when the name lands.
          <div className="min-w-0 flex-1">
            <div className="h-5 w-40 max-w-full rounded-md bg-surface animate-pulse" aria-hidden />
            <span className="sr-only">Loading category</span>
          </div>
        )}
        {!nameless && (
          <Tooltip content="Share category">
            <button
              type="button"
              onClick={share}
              aria-label="Share category"
              className="sn-touch ml-auto shrink-0 flex items-center justify-center text-textSecondary"
            >
              <ShareNetwork size={20} />
            </button>
          </Tooltip>
        )}
      </div>
      {/* Always rendered, so a name that lands later never pushes the list down. */}
      <CategoryTagControls
        name={category.name}
        selected={selected}
        onToggle={toggle}
        pages={pages}
        searchDisabled={nameless}
        searchHidden={nameless && recordState === 'missing'}
      />
      <PullToRefresh onRefresh={refresh} scrollerRef={scroller}>
        {firstLoading ? (
          <SkeletonCards />
        ) : listed.length === 0 ? (
          // No such category: the header already says so, once.
          nameless && recordState === 'missing' && !tagMode ? null : (
            // A failed load keeps its way out: the pull retries it.
            <EmptyNote text={empty} hint={loadFailed && !tagMode ? undefined : null} />
          )
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
    </>
  );
};

// The chip row and the tag sheet, apart from the list: a row answer or opening
// the sheet re-renders only these, never the grid of cards. The sheet portals to
// the page, so it sits here beside the row.
const CategoryTagControls: React.FC<{
  name: string;
  selected: string[];
  onToggle: (tag: string) => void;
  pages: number;
  searchDisabled: boolean;
  searchHidden: boolean;
}> = ({ name, selected, onToggle, pages, searchDisabled, searchHidden }) => {
  const [sheetOpen, setSheetOpen] = useState(false);
  const row = useCategoryTagSuggestions({
    gameName: name,
    query: '',
    selected,
    limit: ROW_LIMIT,
    enabled: true,
    revision: pages,
    row: true,
  });

  return (
    <>
      {/* Fixed under the title, so the filter in use never scrolls away. */}
      <CategoryTagRail
        labels={row?.labels ?? NO_TAGS}
        selected={selected}
        onToggle={onToggle}
        onSearch={() => setSheetOpen(true)}
        searchDisabled={searchDisabled}
        searchHidden={searchHidden}
      />
      <CategoryTagSheet
        open={sheetOpen}
        onClose={() => setSheetOpen(false)}
        gameName={name}
        selected={selected}
        onToggle={onToggle}
        revision={pages}
      />
    </>
  );
};
