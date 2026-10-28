// The rows of Your songs a virtualized list shows. Rust filters, orders and
// pages the history (get_song_history); this holds only the pages near what
// is on screen (songPaging.ts keeps the bookkeeping, tested there), so a long
// history never sits whole in the page, and asks again for what is on screen
// when the history changes in any window.
import { useCallback, useEffect, useEffectEvent, useRef, useState } from 'react';
import { fetchSongPage, onSongHistoryChanged, type SongRow } from '../../utils/songHistory';
import { Logger } from '../../utils/logger';
import {
  emptyPages,
  historyChanged,
  pageLanded,
  pagesFor,
  SONG_PAGE,
  startSearch,
  type PageState,
} from './songPaging';

interface Counts {
  /** Songs the search matches; null until the first page answers. */
  matched: number | null;
  total: number;
  /** Places heard name more than one platform. */
  mixed: boolean;
  /** Channels the songs were heard on. */
  channels: number;
}

export function useSongPages(query: string) {
  // The truth lives in the ref (read by the callbacks below); `pages` mirrors
  // it for render.
  const state = useRef<PageState<SongRow>>(emptyPages());
  const [pages, setPages] = useState<ReadonlyMap<number, SongRow[]>>(() => new Map());
  const [counts, setCounts] = useState<Counts>({ matched: null, total: 0, mixed: false, channels: 0 });
  const [failed, setFailed] = useState(false);
  // Page -> the generation it was asked under, so a page is asked once per generation.
  const inflight = useRef(new Map<number, number>());
  const visible = useRef<number[]>([0]);

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
      fetchSongPage(query, page * SONG_PAGE, SONG_PAGE)
        .then((answer) => {
          const next = pageLanded(state.current, gen, page, answer.rows, visible.current);
          if (!next) return;
          commit(next);
          setCounts({
            matched: answer.matched,
            total: answer.total,
            mixed: answer.mixed_platforms,
            channels: answer.channels,
          });
          setFailed(false);
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
    load(0, true);
  }, [load]);

  // The history changed in any window: ask again for what is on screen. One
  // subscription for the panel's life, so no change slips between searches.
  const onChanged = useEffectEvent(() => {
    commit(historyChanged(state.current, visible.current));
    for (const page of visible.current) load(page, true);
  });
  useEffect(() => {
    const unlisten = onSongHistoryChanged(() => onChanged());
    return () => {
      void unlisten.then((stop) => stop());
    };
  }, []);

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

  return { ...counts, rowAt, ensure, failed, retry };
}
