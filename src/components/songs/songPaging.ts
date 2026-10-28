// Page math for the Your songs list. Pure (no imports), so it is tested
// without a page.

/** Rows asked for per page. */
export const SONG_PAGE = 50;

/** Pages held at most: about what is on screen and either side of it. */
export const KEEP_PAGES = 6;

/** The pages rows `start..=stop` fall on. */
export function pagesFor(start: number, stop: number, size: number = SONG_PAGE): number[] {
  if (stop < start) return [];
  const out: number[] = [];
  for (let page = Math.floor(Math.max(0, start) / size); page <= Math.floor(stop / size); page++) out.push(page);
  return out;
}

/** The held pages to drop so at most `keep` remain, farthest from the pages
 *  on screen first. */
export function pagesToDrop(held: number[], visible: number[], keep: number = KEEP_PAGES): number[] {
  if (held.length <= keep) return [];
  const center = visible.length ? (visible[0] + visible[visible.length - 1]) / 2 : 0;
  return held
    .filter((page) => !visible.includes(page))
    .sort((a, b) => Math.abs(b - center) - Math.abs(a - center))
    .slice(0, held.length - keep);
}

/** What the list holds. Answers from an older search or an older history
 *  are told apart by `gen`; `replace` makes the first answer of a new search
 *  replace the old rows instead of joining them. */
export interface PageState<R> {
  gen: number;
  replace: boolean;
  pages: ReadonlyMap<number, R[]>;
}

export function emptyPages<R>(): PageState<R> {
  return { gen: 0, replace: false, pages: new Map() };
}

/** A new search: older answers no longer count; the old rows stay on screen
 *  until the first new page lands. */
export function startSearch<R>(s: PageState<R>): PageState<R> {
  return { gen: s.gen + 1, replace: true, pages: s.pages };
}

/** The history changed: older answers no longer count, and only the pages on
 *  screen are kept until they are asked for again. */
export function historyChanged<R>(s: PageState<R>, visible: number[]): PageState<R> {
  return { gen: s.gen + 1, replace: s.replace, pages: new Map([...s.pages].filter(([page]) => visible.includes(page))) };
}

/** A page answered under `gen`; null when that answer no longer counts. */
export function pageLanded<R>(
  s: PageState<R>,
  gen: number,
  page: number,
  rows: R[],
  visible: number[],
  keep: number = KEEP_PAGES,
): PageState<R> | null {
  if (gen !== s.gen) return null;
  const pages = new Map(s.replace ? [] : s.pages);
  pages.set(page, rows);
  for (const drop of pagesToDrop([...pages.keys()], visible, keep)) pages.delete(drop);
  return { gen: s.gen, replace: false, pages };
}
