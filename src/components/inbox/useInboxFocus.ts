// Shows a pinned mention's entry in an inbox list (SN-284). A pin from another
// chat asks the inbox to open at its entry (`requestMentionFocus`); the list
// that is on screen finds the row (loading older pages while it is not among
// the loaded ones, a few times), scrolls it to the middle and highlights it for
// a moment. The row keeps its own opener. A list that never finds it gives up
// quietly after FOCUS_MAX_PAGES pages.

import { useEffect, useRef, useState, type RefObject } from 'react';
import {
  FOCUS_HIGHLIGHT_MS,
  FOCUS_MAX_PAGES,
  settleMentionFocus,
  useMentionFocus,
} from '../../stores/mentionFocusStore';
import type { InboxItem } from '../../types';

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
  loadOlder: () => void;
  loaded: boolean;
  container: RefObject<HTMLElement | null>;
  active: boolean;
}): string | null {
  const { rows, hasMore, loadOlder, loaded, container, active } = opts;
  const focus = useMentionFocus();
  const [focusedRowId, setFocusedRowId] = useState<string | null>(null);
  const pages = useRef<{ nonce: number; asked: number; at: number }>({ nonce: -1, asked: 0, at: -1 });

  useEffect(() => {
    if (!focus || !active || !loaded) return;
    const rowId = rowHolding(rows, focus.id);
    if (rowId) {
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
    if (hasMore && state.asked < FOCUS_MAX_PAGES && state.at !== rows.length) {
      state.asked += 1;
      state.at = rows.length;
      loadOlder();
      return;
    }
    if (!hasMore || state.asked >= FOCUS_MAX_PAGES) settleMentionFocus(focus.nonce);
  }, [focus, active, loaded, rows, hasMore, loadOlder, container]);

  return focusedRowId;
}
