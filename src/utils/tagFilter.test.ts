// The tag filter's client rules (SN-78): picks, the filtered list's pages, which
// answer from Rust shows, the one-retry rule, a pill's click order, and the
// desktop's keep-or-clear rule for its stored picks.
import { describe, expect, it } from 'vitest';
import type { TwitchStream } from '../types';
import {
  acceptAnswer,
  applyPage,
  beginMore,
  categoryKey,
  categoryTagsFor,
  failPage,
  hasTag,
  newTagList,
  NO_RETRY,
  NO_TAGS,
  pillClick,
  refreshTagList,
  retryStep,
  tagsKey,
  tagView,
  toggleTag,
  type TagAnswer,
} from './tagFilter';

const row = (id: string, user = id) => ({ id, user_id: user }) as unknown as TwitchStream;
const ids = (streams: TwitchStream[]) => streams.map((s) => s.id);

describe('picks', () => {
  it('toggles case-insensitively and trims', () => {
    expect(toggleTag([], ' English ')).toEqual(['English']);
    expect(toggleTag(['English'], 'english')).toEqual([]);
    expect(toggleTag(['A'], '   ')).toEqual(['A']);
    expect(hasTag(['English'], 'ENGLISH ')).toBe(true);
    expect(hasTag(NO_TAGS, 'English')).toBe(false);
  });

  it('keys a set whatever its order or case, and keeps joined names apart', () => {
    expect(tagsKey(['b', 'A'])).toBe(tagsKey(['a', 'B']));
    expect(tagsKey(['ab', 'c'])).not.toBe(tagsKey(['a', 'bc']));
    expect(categoryKey('  Just Chatting ')).toBe('just chatting');
    expect(categoryKey(undefined)).toBe('');
  });
});

describe('tag list', () => {
  it('drops a page asked for older picks, even one that lands first', () => {
    let list = newTagList(2, true);
    list = applyPage(list, 1, [row('old')], 'c', false, 320);
    expect(list.streams).toEqual([]);
    expect(list.loading).toBe(true);
    list = applyPage(list, 2, [row('a')], 'c2', false, 320);
    expect(ids(list.streams)).toEqual(['a']);
    expect(list).toMatchObject({ loading: false, hasMore: true, cursor: 'c2' });
  });

  it('appends only new channels, stops at the cap and clears the busy flag', () => {
    let list = applyPage(newTagList(1, true), 1, [row('a'), row('b')], 'c', false, 3);
    list = beginMore(list, 1);
    expect(list.loadingMore).toBe(true);
    list = applyPage(list, 1, [row('b2', 'b'), row('c'), row('d')], 'c3', true, 3);
    expect(ids(list.streams)).toEqual(['a', 'b', 'c']);
    expect(list).toMatchObject({ loadingMore: false, hasMore: false });
  });

  it('a stale failure never clears the current busy flag', () => {
    let list = beginMore(applyPage(newTagList(5, true), 5, [row('a')], 'c', false, 320), 5);
    list = failPage(list, 4, true);
    expect(list.loadingMore).toBe(true);
    list = failPage(list, 5, true);
    expect(list).toMatchObject({ loadingMore: false, hasMore: false });
    expect(ids(list.streams)).toEqual(['a']);
  });

  it('a refresh keeps its rows until its first page, and a failed refresh keeps them', () => {
    const shown = applyPage(newTagList(1, true), 1, [row('a')], 'c', false, 320);
    const refreshing = refreshTagList(shown, 2);
    expect(ids(refreshing.streams)).toEqual(['a']);
    expect(refreshing.hasMore).toBe(false);
    expect(applyPage(refreshing, 1, [row('x')], null, true, 320)).toBe(refreshing);
    expect(ids(failPage(refreshing, 2, false).streams)).toEqual(['a']);
    expect(ids(applyPage(refreshing, 2, [row('b')], null, false, 320).streams)).toEqual(['b']);
  });

  it('an empty page or no cursor ends paging', () => {
    expect(applyPage(newTagList(1, true), 1, [], 'c', false, 320).hasMore).toBe(false);
    expect(applyPage(newTagList(1, true), 1, [row('a')], null, false, 320).hasMore).toBe(false);
  });
});

describe('answers', () => {
  const answer = (gameName: string, query: string): TagAnswer => ({
    labels: ['x'],
    offer_typed: null,
    retry_after_ms: null,
    gameName,
    query,
  });

  it('keeps only the newest ask', () => {
    expect(acceptAnswer(3, 3)).toBe(true);
    expect(acceptAnswer(2, 3)).toBe(false);
  });

  it('draws an answer only for the category on screen, and marks the one for this text', () => {
    const a = answer('Art', 'sp');
    expect(tagView(a, 'Art', 'sp')).toEqual({ labels: ['x'], typed: null, current: true });
    expect(tagView(a, 'Chess', 'sp')).toBeNull();
    expect(tagView(null, 'Art', 'sp')).toBeNull();
  });

  it('keeps the last answer drawn while newer text is asked, the typed row in place', () => {
    // No typed row before (an exact match): none until the new answer lands.
    expect(tagView(answer('Art', 'sp'), 'Art', 'spe')).toEqual({ labels: ['x'], typed: null, current: false });
    // A typed row before: it stays and shows the text on screen.
    const offered = { ...answer('Art', 'sp'), offer_typed: 'sp' };
    expect(tagView(offered, 'Art', ' spe ')).toEqual({ labels: ['x'], typed: 'spe', current: false });
    // The first key after an empty box shows the row at once.
    expect(tagView(answer('Art', ''), 'Art', 'a')?.typed).toBe('a');
    // A cleared box never offers an empty tag.
    expect(tagView(offered, 'Art', '')).toEqual({ labels: ['x'], typed: null, current: false });
  });

  it('retries once per revision, and a retry never re-arms itself', () => {
    const failing = { labels: [], offer_typed: null, retry_after_ms: 30000 };
    const first = retryStep(NO_RETRY, '1', failing);
    expect(first.delay).toBe(30000);
    const fromRetry = retryStep(first.state, '1', failing);
    expect(fromRetry.delay).toBeNull();
    expect(retryStep(fromRetry.state, '2', failing).delay).toBe(30000);
    expect(retryStep(NO_RETRY, '1', { ...failing, retry_after_ms: null }).delay).toBeNull();
  });
});

describe('pill click', () => {
  it('stops the click before reading the hold, and toggles only without one', () => {
    const calls: string[] = [];
    pillClick({
      stop: () => calls.push('stop'),
      consumeHeld: () => {
        calls.push('held?');
        return false;
      },
      toggle: () => calls.push('toggle'),
    });
    expect(calls).toEqual(['stop', 'held?', 'toggle']);
    const afterHold: string[] = [];
    pillClick({ stop: () => afterHold.push('stop'), consumeHeld: () => true, toggle: () => afterHold.push('toggle') });
    expect(afterHold).toEqual(['stop']);
  });
});

describe('desktop picks', () => {
  const stored = { key: 'art', tags: ['English'] };

  it('keeps them in the category and on a search started from it', () => {
    expect(categoryTagsFor({ activeTab: 'category', searchReturnTab: 'following', categoryKey: 'art', stored })).toEqual({
      tags: ['English'],
      clear: false,
    });
    expect(categoryTagsFor({ activeTab: 'search', searchReturnTab: 'category', categoryKey: 'art', stored }).clear).toBe(false);
  });

  it('clears them on another tab, another search or another category', () => {
    expect(categoryTagsFor({ activeTab: 'browse', searchReturnTab: 'category', categoryKey: 'art', stored }).clear).toBe(true);
    expect(categoryTagsFor({ activeTab: 'search', searchReturnTab: 'following', categoryKey: 'art', stored }).clear).toBe(true);
    expect(categoryTagsFor({ activeTab: 'category', searchReturnTab: 'following', categoryKey: 'chess', stored })).toEqual({
      tags: NO_TAGS,
      clear: true,
    });
  });

  it('has nothing to clear once cleared', () => {
    const empty = { key: '', tags: [] };
    expect(categoryTagsFor({ activeTab: 'browse', searchReturnTab: 'browse', categoryKey: '', stored: empty }).clear).toBe(false);
  });
});
