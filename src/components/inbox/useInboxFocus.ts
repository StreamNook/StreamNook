// Shows a pinned mention's entry in an inbox list (SN-284). A pin from another
// chat asks the inbox to open at its entry (`requestMentionFocus`); the list
// that is on screen finds the row (loading older pages while it is not among
// the loaded ones, up to the end of the list), scrolls it to the middle and
// highlights it for a moment. The row keeps its own opener. A list that does not
// have it by its last page says so, so the tap is never left looking dead.

import { useEffect, useRef, useState, type RefObject } from 'react';
import {
  FOCUS_HIGHLIGHT_MS,
  FOCUS_MAX_PAGES,
  settleMentionFocus,
  useMentionFocus,
} from '../../stores/mentionFocusStore';
import { useAppStore } from '../../stores/AppStore';
import type { InboxItem } from '../../types';

/** What the viewer is told when the entry is no longer in the inbox. */
export const FOCUS_MISSING_WORDS = 'That mention is no longer in your inbox';

/** The ids a mention row stands for: its newest entry and every folded copy. */
export function mentionRowIdsOf(row: InboxItem): string[] {
  return row.kind === 'mention' ? [row.entry.id, ...row.folded.map((f) => f.id)] : [];
}

/** The row_id of the first mention row that holds `id`, or null. */
export function rowHolding(rows: readonly InboxItem[], id: string): string | null {
  for (const r of rows) {
    if (r.kind === 'mention' && mentionRowIdsOf(r).includes(id)) return r.row_id;
  }
  return null;
}

/**
 * `focusedRowId` is the row to draw highlighted (null when none). `container`
 * holds the rows; each mention row carries `data-mention-ids` (its ids,
 * space-separated). `active` is whether this list is on screen.
 */
export function useInboxFocus(opts: {
  rows: readonly InboxItem[];
  hasMore: boolean;
  /** Asks for the next older page. A list that can say how it went answers
   *  whether the page added rows: false for a failed read or one with nothing
   *  new, so the search never waits on a list that will not change. */
  loadOlder: () => void | Promise<boolean>;
  loaded: boolean;
  container: RefObject<HTMLElement | null>;
  active: boolean;
}): string | null {
  const { rows, hasMore, loadOlder, loaded, container, active } = opts;
  const focus = useMentionFocus();
  const [focusedRowId, setFocusedRowId] = useState<string | null>(null);
  const pages = useRef<{ nonce: number; asked: number; at: number; told: number }>({ nonce: -1, asked: 0, at: -1, told: -1 });

  useEffect(() => {
    if (!focus || !active || !loaded) return;
    const rowId = rowHolding(rows, focus.id);
    if (rowId) {
      // Found: a page still in flight has nothing left to say.
      pages.current.told = focus.nonce;
      setFocusedRowId(rowId);
      const el = container.current?.querySelector<HTMLElement>(`[data-mention-ids~="${CSS.escape(focus.id)}"]`);
      el?.scrollIntoView({ block: 'center', behavior: 'smooth' });
      const nonce = focus.nonce;
      const timer = setTimeout(() => {
        setFocusedRowId(null);
        settleMentionFocus(nonce);
      }, FOCUS_HIGHLIGHT_MS);
      return () => clearTimeout(timer);
    }
    // Not among the loaded rows: one more page per change of what is loaded.
    const state = pages.current;
    if (state.nonce !== focus.nonce) {
      state.nonce = focus.nonce;
      state.asked = 0;
      state.at = -1;
    }
    // Says it once per request, whichever way the search ends.
    const giveUp = (n: number) => {
      if (state.told === n) return;
      state.told = n;
      useAppStore.getState().addToast(FOCUS_MISSING_WORDS, 'info');
      settleMentionFocus(n);
    };
    if (hasMore && state.asked < FOCUS_MAX_PAGES && state.at !== rows.length) {
      state.asked += 1;
      state.at = rows.length;
      const nonce = focus.nonce;
      const answer = loadOlder();
      // A page that failed or added nothing leaves the list as it was, so nothing
      // would ask again: the search ends here instead of waiting for ever.
      if (answer && typeof answer.then === 'function') {
        answer.then(
          (grew) => {
            if (!grew) giveUp(nonce);
          },
          () => giveUp(nonce),
        );
      }
      return;
    }
    if (!hasMore || state.asked >= FOCUS_MAX_PAGES) giveUp(focus.nonce);
  }, [focus, active, loaded, rows, hasMore, loadOlder, container]);

  return focusedRowId;
}
