// The rows of Your songs a virtualized list shows. Rust filters, orders and
// pages the history (get_song_history); this holds only the pages near what
// is on screen (songPaging.ts keeps the bookkeeping, tested there), so a long
// history never sits whole in the page, and asks again for what is on screen
// when the history changes in any window.
//
// It also keeps the list's time groups: the bounds are made here in local
// time (songGroups.ts boundsFor) and sent with every page of the unsearched
// list, and Rust answers where each group starts. And it holds a song filed
// to the top while the list is scrolled away from its top (songGroups.ts
// holdChange): the rows on screen stay, and the panel shows a "New song" pill.
import { useCallback, useEffect, useEffectEvent, useRef, useState } from 'react';
import {
  fetchSongPage,
  onSongHistoryChanged,
  type SongHistoryChange,
  type SongRow,
} from '../../utils/songHistory';
import { Logger } from '../../utils/logger';
import { onWindowVisibility, isWindowHidden } from '../../utils/windowVisibility';
import {
  emptyPages,
  historyChanged,
  pageLanded,
  pagesFor,
  SONG_PAGE,
  startSearch,
  type PageState,
} from './songPaging';
import {
  boundsFor,
  filedToTop,
  heldCount,
  holdChange,
  msToNextDay,
  NOTHING_HELD,
  releaseHeld,
  sameBounds,
  type Bounds,
  type HeldChanges,
  type SongGroup,
  type TopRow,
} from './songGroups';

interface Counts {
  /** Songs the search matches; null until the first page answers. */
  matched: number | null;
  total: number;
  /** Places heard name more than one platform. */
  mixed: boolean;
  /** Channels the songs were heard on. */
  channels: number;
}

/** The time groups of the matched songs, with the names of the bounds they
 *  were counted against. */
export interface Grouping {
  groups: SongGroup[];
  labels: string[];
}

const NO_GROUPING: Grouping = { groups: [], labels: [] };

const sameGroups = (a: SongGroup[], b: SongGroup[]): boolean =>
  a.length === b.length && a.every((g, i) => g.bound === b[i].bound && g.start === b[i].start);

/** The bounds for `now` when they differ from `current`, else null (the list
 *  keeps its bounds, and asks nothing again). */
export function boundsMovedOn(current: Bounds, now: Date): Bounds | null {
  const next = boundsFor(now);
  return sameBounds(current, next) ? null : next;
}

/** What a change from Rust does to the list. */
export interface ChangeStep {
  held: HeldChanges;
  /** Ask again for the rows on screen now. */
  apply: boolean;
  /** The song is new to the history and waits at its top: pages asked for
   *  meanwhile are read one row further on. */
  shifts: boolean;
}

/**
 * A change, with the list scrolled away from its top or not. For a song filed
 * while scrolled, `peek` is the history's first row before and after it (null
 * when Rust could not be asked): only a song that went to the top waits behind
 * the pill; a song changed in place (its links looked up again, an Undo
 * putting it back where it was) applies at once, so the pill never shows for
 * nothing.
 */
export function stepForChange(
  held: HeldChanges,
  change: SongHistoryChange,
  scrolled: boolean,
  peek: { before: TopRow | null; after: (TopRow & { count?: number }) | null } | null,
): ChangeStep {
  let toTop = true;
  let fresh = false;
  if (change.kind === 'filed' && scrolled) {
    toTop = !!peek && filedToTop(change.key, peek.before, peek.after);
    // A song never heard before: one more row at the top of Rust's list.
    fresh = toTop && peek?.before?.key !== change.key && peek?.after?.count === 1;
  }
  const step = holdChange(held, change, scrolled, toTop);
  const waiting = change.kind === 'filed' && held.filed.includes(change.key);
  return { ...step, shifts: !step.apply && fresh && !waiting };
}

export function useSongPages(query: string) {
  // The group bounds, kept while their values hold: made once as the list
  // opens, and again from the clock (never from the old "now") on each page
  // answer, at the next local midnight, when the window comes back into view
  // and when it takes focus (timers run late after a sleep).
  const [openedWith] = useState(() => boundsFor(new Date()));
  const bounds = useRef<Bounds>(openedWith);
  // The truth lives in the ref (read by the callbacks below); `pages` mirrors
  // it for render.
  const state = useRef<PageState<SongRow>>(emptyPages());
  const [pages, setPages] = useState<ReadonlyMap<number, SongRow[]>>(() => new Map());
  const [counts, setCounts] = useState<Counts>({ matched: null, total: 0, mixed: false, channels: 0 });
  const [grouping, setGrouping] = useState<Grouping>(NO_GROUPING);
  const [failed, setFailed] = useState(false);
  // Page -> the generation it was asked under, so a page is asked once per generation.
  const inflight = useRef(new Map<number, number>());
  const visible = useRef<number[]>([0]);

  // Changes held while the list is scrolled away from its top.
  const scrolled = useRef(false);
  const heldRef = useRef<HeldChanges>(NOTHING_HELD);
  // What the pill counts, for the search it was held in (a new search lets
  // everything go).
  const [heldShown, setHeldShown] = useState({ query, count: 0 });
  // Songs new to the history among those held: Rust's list now starts with
  // them, so a page asked for meanwhile is read that much further on and
  // still lines up with the rows on screen.
  const shift = useRef(0);
  // The history's first row as last seen, to tell a song filed to the top from
  // one changed in place (its links looked up again).
  const top = useRef<TopRow | null>(null);
  const setHeld = useCallback(
    (next: HeldChanges) => {
      heldRef.current = next;
      if (!next.filed.length && !next.more) shift.current = 0;
      setHeldShown({ query, count: heldCount(next) });
    },
    [query],
  );

  const commit = useCallback((next: PageState<SongRow>) => {
    state.current = next;
    setPages(next.pages);
  }, []);

  const load = useCallback(
    (page: number, force = false) => {
      const now = state.current;
      if (!force && now.pages.has(page)) return;
      if (inflight.current.get(page) === now.gen) return;
      const gen = now.gen;
      inflight.current.set(page, gen);
      const asked = query ? null : bounds.current;
      fetchSongPage(query, page * SONG_PAGE + shift.current, SONG_PAGE, asked?.ms)
        .then((answer) => {
          const next = pageLanded(state.current, gen, page, answer.rows, visible.current);
          if (!next) return;
          commit(next);
          setFailed(false);
          if (page === 0 && !shift.current) top.current = answer.rows[0] ?? null;
          // While a song waits, the counts and groups on screen stay as they
          // are: they would already count it.
          if (heldRef.current.filed.length || heldRef.current.more) return;
          setCounts({
            matched: answer.matched,
            total: answer.total,
            mixed: answer.mixed_platforms,
            channels: answer.channels,
          });
          const groups = asked ? (answer.groups ?? []) : [];
          const labels = asked ? asked.labels : [];
          setGrouping((was) => (sameGroups(was.groups, groups) && was.labels === labels ? was : { groups, labels }));
        })
        .catch((e: unknown) => {
          Logger.warn('[Songs] page failed:', e);
          if (gen === state.current.gen) setFailed(true);
        })
        .finally(() => {
          if (inflight.current.get(page) === gen) inflight.current.delete(page);
        });
    },
    [query, commit],
  );

  // A new search starts from the top. The rows on screen stay until its first
  // page lands, so only the bookkeeping moves on here.
  useEffect(() => {
    state.current = startSearch(state.current);
    visible.current = [0];
    top.current = null;
    heldRef.current = NOTHING_HELD;
    shift.current = 0;
    load(0, true);
  }, [load]);

  // New bounds (the day moved on) start a new generation of the unsearched
  // list: its rows stay until the first page asked under them lands with its
  // groups.
  const checkBounds = useEffectEvent(() => {
    const next = boundsMovedOn(bounds.current, new Date());
    if (!next) return;
    bounds.current = next;
    if (query) return;
    state.current = startSearch(state.current);
    for (const page of visible.current) load(page, true);
  });
  // Each page answer brings new counts.
  useEffect(() => checkBounds(), [counts]);
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const arm = () => {
      clearTimeout(timer);
      // A beat past midnight, so the new day has begun when it fires.
      timer = setTimeout(() => {
        checkBounds();
        arm();
      }, msToNextDay(new Date()) + 1000);
    };
    arm();
    const wake = () => {
      if (isWindowHidden()) return;
      checkBounds();
      arm();
    };
    const stopVisibility = onWindowVisibility(wake);
    window.addEventListener('focus', wake);
    return () => {
      clearTimeout(timer);
      stopVisibility();
      window.removeEventListener('focus', wake);
    };
  }, []);

  /** Ask again for what is on screen. */
  const applyNow = useCallback(() => {
    commit(historyChanged(state.current, visible.current));
    for (const page of visible.current) load(page, true);
  }, [commit, load]);

  // Filed changes are read in order: each one asks Rust for the list's first
  // row to see whether the song went to the top.
  const queue = useRef<Promise<void>>(Promise.resolve());
  const onChanged = useEffectEvent((change: SongHistoryChange) => {
    queue.current = queue.current.then(async () => {
      let peek: Parameters<typeof stepForChange>[3] = null;
      if (change.kind === 'filed' && scrolled.current) {
        try {
          const answer = await fetchSongPage(query, 0, 1, query ? undefined : bounds.current.ms);
          peek = { before: top.current, after: answer.rows[0] ?? null };
          top.current = peek.after;
        } catch (e) {
          // Unknown: it applies, rather than a pill for nothing.
          Logger.warn('[Songs] first row failed:', e);
        }
      }
      const step = stepForChange(heldRef.current, change, scrolled.current, peek);
      if (step.shifts) shift.current += 1;
      setHeld(step.held);
      if (step.apply) applyNow();
    });
  });
  // One subscription for the panel's life, so no change slips between searches.
  useEffect(() => {
    const unlisten = onSongHistoryChanged((change) => onChanged(change));
    return () => {
      void unlisten.then((stop) => stop());
    };
  }, []);

  /** Apply what waits (the pill, or the list back at its top). */
  const release = useCallback(() => {
    const step = releaseHeld(heldRef.current);
    setHeld(step.held);
    if (step.apply) applyNow();
  }, [applyNow, setHeld]);

  /** The list moved: away from its top, or back to it (which applies what
   *  waits). */
  const setScrolled = useCallback(
    (away: boolean) => {
      if (scrolled.current === away) return;
      scrolled.current = away;
      if (!away && (heldRef.current.filed.length > 0 || heldRef.current.more)) release();
    },
    [release],
  );

  const ensure = useCallback(
    (start: number, stop: number) => {
      const want = pagesFor(start, stop);
      visible.current = want;
      for (const page of want) load(page);
    },
    [load],
  );

  const retry = useCallback(() => {
    setFailed(false);
    for (const page of visible.current) load(page, true);
  }, [load]);

  const rowAt = useCallback(
    (index: number): SongRow | null => pages.get(Math.floor(index / SONG_PAGE))?.[index % SONG_PAGE] ?? null,
    [pages],
  );

  /** Songs the "New song" pill counts. */
  const held = heldShown.query === query ? heldShown.count : 0;

  return { ...counts, ...grouping, rowAt, ensure, failed, retry, held, release, setScrolled };
}
