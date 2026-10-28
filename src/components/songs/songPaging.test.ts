import { describe, expect, it } from 'vitest';
import { emptyPages, historyChanged, pageLanded, pagesFor, pagesToDrop, startSearch, type PageState } from './songPaging';

/** A landed page that must count (fails the test when it does not). */
function landed<R>(next: PageState<R> | null): PageState<R> {
  expect(next).not.toBeNull();
  return next as PageState<R>;
}

describe('song list paging', () => {
  it('names the pages a row range falls on', () => {
    expect(pagesFor(0, 0, 50)).toEqual([0]);
    expect(pagesFor(45, 105, 50)).toEqual([0, 1, 2]);
    expect(pagesFor(10, 5, 50)).toEqual([]);
    expect(pagesFor(-3, 2, 50)).toEqual([0]);
  });

  it('drops the pages farthest from the screen first', () => {
    expect(pagesToDrop([0, 1, 2], [1], 6)).toEqual([]);
    expect(pagesToDrop([0, 1, 2, 3, 4, 5, 6, 7], [6, 7], 6).sort((a, b) => a - b)).toEqual([0, 1]);
    expect(pagesToDrop([0, 9, 10, 11, 12, 13, 14], [10], 6)).toEqual([0]);
  });

  it('ignores an answer from before a new search, and the first new one replaces the rows', () => {
    let s = landed(pageLanded(emptyPages<string>(), 0, 0, ['a'], [0]));
    s = landed(pageLanded(s, 0, 1, ['b'], [0, 1]));
    const searching = startSearch(s);
    expect(pageLanded(searching, s.gen, 1, ['stale'], [0])).toBeNull();
    const first = landed(pageLanded(searching, searching.gen, 0, ['x'], [0]));
    expect([...first.pages.entries()]).toEqual([[0, ['x']]]);
  });

  it('keeps only the pages on screen when the history changes, and still replaces after a search', () => {
    let s = emptyPages<string>();
    for (const p of [0, 1, 2]) s = landed(pageLanded(s, s.gen, p, [`p${p}`], [0, 1, 2]));
    const changed = historyChanged(s, [1]);
    expect([...changed.pages.keys()]).toEqual([1]);
    expect(pageLanded(changed, s.gen, 0, ['old'], [1])).toBeNull();
    const both = historyChanged(startSearch(s), [0]);
    expect(both.replace).toBe(true);
    expect([...landed(pageLanded(both, both.gen, 0, ['new'], [0])).pages.keys()]).toEqual([0]);
  });
});
