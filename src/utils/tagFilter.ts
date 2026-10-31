// The tag filter's client rules, shared by the desktop category page and the
// Android category screen (the iPhone has the same rules in SNKit TagFilter).
// Rust ranks and pools the tags (category_tag_suggestions); what lives here is
// each screen's own state: which tags are picked, the filtered list's pages,
// and which answer from Rust is still the one to show.
import type { TwitchStream } from '../types';

/** No picks: one shared empty list, so code that depends on it does not see a
 *  new array every render. */
export const NO_TAGS: readonly string[] = Object.freeze([]);

/** Adds the tag, or removes it when it is already picked (case does not matter). */
export function toggleTag(selected: readonly string[], label: string): string[] {
  const tag = label.trim();
  if (!tag) return [...selected];
  const key = tag.toLowerCase();
  return selected.some((t) => t.toLowerCase() === key)
    ? selected.filter((t) => t.toLowerCase() !== key)
    : [...selected, tag];
}

/** Whether the tag is picked (case does not matter). Every screen draws a
 *  chip's selected state from this, never from Rust's answer. */
export function hasTag(selected: readonly string[], label: string): boolean {
  const key = label.trim().toLowerCase();
  return selected.some((t) => t.toLowerCase() === key);
}

/** One key per set of picks, whatever their order or case. The separator keeps
 *  ["ab", "c"] and ["a", "bc"] apart. */
export function tagsKey(selected: readonly string[]): string {
  return selected
    .map((t) => t.trim().toLowerCase())
    .sort()
    .join('\u001f');
}

/** A category's key for its picks: its name, trimmed and lowercased (the same
 *  key Rust files the category's tags under). */
export function categoryKey(name: string | null | undefined): string {
  return (name ?? '').trim().toLowerCase();
}

/** The filtered list for one set of picks. `gen` names the ask it belongs to;
 *  a page asked for older picks never lands on it. */
export interface TagList {
  gen: number;
  streams: TwitchStream[];
  cursor: string | null;
  hasMore: boolean;
  loading: boolean;
  loadingMore: boolean;
}

/** A fresh list for a new set of picks (`active` false: no picks). */
export function newTagList(gen: number, active: boolean): TagList {
  return { gen, streams: [], cursor: null, hasMore: false, loading: active, loadingMore: false };
}

/** Pull to refresh with the same picks: a new generation that keeps the rows on
 *  screen until its first page replaces them, as the plain list does. */
export function refreshTagList(list: TagList, gen: number): TagList {
  return { ...list, gen, hasMore: false, loading: false, loadingMore: false };
}

/** The next page is on its way. */
export function beginMore(list: TagList, gen: number): TagList {
  if (gen !== list.gen) return list;
  return { ...list, loadingMore: true };
}

function rowId(s: TwitchStream): string {
  return s.user_id || s.id;
}

/** A page arrived. The first page replaces; a later one adds only channels the
 *  list lacks (live counts reorder under a cursor), up to `cap`. */
export function applyPage(
  list: TagList,
  gen: number,
  page: TwitchStream[],
  cursor: string | null,
  append: boolean,
  cap: number,
): TagList {
  if (gen !== list.gen) return list;
  let streams: TwitchStream[];
  if (append) {
    const seen = new Set(list.streams.map(rowId));
    streams = [...list.streams, ...page.filter((s) => !seen.has(rowId(s)))].slice(0, cap);
  } else {
    streams = page.slice(0, cap);
  }
  return {
    gen,
    streams,
    cursor,
    hasMore: !!cursor && page.length > 0 && streams.length < cap,
    loading: append ? list.loading : false,
    loadingMore: append ? false : list.loadingMore,
  };
}

/** A page failed: paging stops and the rows already shown stay (a failed
 *  refresh keeps its list); pulling again retries. */
export function failPage(list: TagList, gen: number, append: boolean): TagList {
  if (gen !== list.gen) return list;
  return append ? { ...list, hasMore: false, loadingMore: false } : { ...list, hasMore: false, loading: false };
}

/** What Rust answers `category_tag_suggestions` with. */
export interface TagSuggestions {
  labels: string[];
  offer_typed: string | null;
  retry_after_ms: number | null;
}

/** An answer with the ask it belongs to. */
export interface TagAnswer extends TagSuggestions {
  gameName: string;
  query: string;
}

/** Only the newest ask's answer is kept. */
export function acceptAnswer(reqId: number, latestId: number): boolean {
  return reqId === latestId;
}

/** What a tag list draws. */
export interface TagView {
  labels: string[];
  /** The "Filter by tag" text, or `null` for no such row. */
  typed: string | null;
  /** The answer is for exactly the text on screen. Only Enter needs this. */
  current: boolean;
}

/** The newest answer for this category stays drawn while an ask for newer text
 *  is in flight, so the list never blanks between keystrokes. Until the new
 *  answer lands, the "Filter by tag" row keeps its place and shows the text on
 *  screen, so no row appears or goes per key. */
export function tagView(answer: TagAnswer | null, gameName: string, query: string): TagView | null {
  if (!answer || answer.gameName !== gameName) return null;
  if (answer.query === query) return { labels: answer.labels, typed: answer.offer_typed, current: true };
  const text = query.trim();
  const keepsRow = answer.offer_typed !== null || answer.query.trim() === '';
  return { labels: answer.labels, typed: text && keepsRow ? text : null, current: false };
}

/** The one-retry rule for a category whose own tags could not be read. `key`
 *  names the revision (the category and its landed pages); a new key allows one
 *  more retry, so a failing category costs at most one extra ask per page and
 *  never polls. */
export interface RetryState {
  key: string;
  retried: boolean;
}

export const NO_RETRY: RetryState = { key: '', retried: false };

/** An accepted answer for revision `key`: the delay to arm (or `null`) and the
 *  state after it. A retry's own answer has the same key, so it never re-arms. */
export function retryStep(state: RetryState, key: string, answer: TagSuggestions): { state: RetryState; delay: number | null } {
  const current = state.key === key ? state : { key, retried: false };
  if (current.retried || answer.retry_after_ms == null) return { state: current, delay: null };
  return { state: { key, retried: true }, delay: answer.retry_after_ms };
}

/** A tag pill on a card: its click is stopped first, so the card under it never
 *  sees it, then a click that ends a hold does nothing (the hold sheet opened). */
export function pillClick(steps: { stop: () => void; consumeHeld: () => boolean; toggle: () => void }): void {
  steps.stop();
  if (steps.consumeHeld()) return;
  steps.toggle();
}

/** The desktop's stored picks: one category's tags, kept while you are in it. */
export interface StoredCategoryTags {
  key: string;
  tags: string[];
}

/** Which picks the desktop category page uses, and whether the stored ones must
 *  go. A search started from the category is still inside it (Escape returns
 *  there); any other tab, or another category, leaves it. */
export function categoryTagsFor(args: {
  activeTab: string;
  searchReturnTab: string;
  categoryKey: string;
  stored: StoredCategoryTags;
}): { tags: readonly string[]; clear: boolean } {
  const { activeTab, searchReturnTab, categoryKey: key, stored } = args;
  const inside = activeTab === 'category' || (activeTab === 'search' && searchReturnTab === 'category');
  if (inside && !!key && stored.key === key) return { tags: stored.tags, clear: false };
  return { tags: NO_TAGS, clear: stored.tags.length > 0 || stored.key !== '' };
}
