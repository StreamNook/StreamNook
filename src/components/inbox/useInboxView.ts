// One inbox view for a desktop surface (the island, MultiChat's Mentions
// mode): Rust's `get_inbox_view` a page at a time, read again whenever Rust
// says something in it changed. Rust owns the rows, their read state and the
// counts; this keeps the loaded pages, and lets the surface show the viewer's
// change at once while Rust confirms it.
import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Logger } from '../../utils/logger';
import {
  CHANNEL_AVATARS_EVENT,
  INBOX_CHANGED_EVENT,
  MENTION_INBOX_CHANGED_EVENT,
  WHISPER_ARCHIVE_CHANGED_EVENT,
  WHISPER_UPDATED_EVENT,
  acceptInboxCounts,
} from '../../stores/mentionInboxStore';
import type { InboxItem, InboxSurface, InboxViewPayload } from '../../types';

export const INBOX_PAGE = 20;
/** `get_inbox_view` caps a page here. */
export const INBOX_PAGE_MAX = 200;
/** Changes come in bursts in a busy chat, so they are gathered for a moment. */
const RELOAD_DELAY_MS = 120;
const RELOAD_EVENTS = [
  MENTION_INBOX_CHANGED_EVENT,
  WHISPER_UPDATED_EVENT,
  WHISPER_ARCHIVE_CHANGED_EVENT,
  INBOX_CHANGED_EVENT,
  CHANNEL_AVATARS_EVENT,
];

export interface InboxViewState {
  rows: InboxItem[];
  hasMore: boolean;
  /** The first answer came back (or failed). */
  loaded: boolean;
  /** The next older page, when there is one. */
  loadOlder: () => void;
  /** Show the viewer's change at once; the next read from Rust replaces it. */
  patch: (fn: (rows: InboxItem[]) => InboxItem[]) => void;
}

function seedCounts(view: InboxViewPayload): void {
  // The view carries Rust's whole published snapshot, so every count (the
  // Whispers window reads `whispers`) is right from the first answer.
  acceptInboxCounts(view.counts);
}

export function useInboxView(surface: InboxSurface, enabled: boolean): InboxViewState {
  const [rows, setRows] = useState<InboxItem[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const cursor = useRef<string | null>(null);
  const loadedCount = useRef(INBOX_PAGE);
  const loadingOlder = useRef(false);

  useEffect(() => {
    loadedCount.current = Math.max(INBOX_PAGE, rows.length);
  }, [rows.length]);

  // The first pages again, as many as are loaded, after every change Rust
  // reports (a new mention or whisper, a read, a removal, an account switch).
  useEffect(() => {
    if (!enabled) return;
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const reload = () => {
      invoke<InboxViewPayload>('get_inbox_view', {
        filter: 'all',
        surface,
        limit: Math.min(loadedCount.current, INBOX_PAGE_MAX),
      })
        .then((view) => {
          if (!alive) return;
          setRows(view.rows);
          setHasMore(view.has_more);
          cursor.current = view.cursor;
          setLoaded(true);
          seedCounts(view);
        })
        .catch((e) => {
          Logger.debug('[Inbox] get_inbox_view failed:', e);
          if (alive) setLoaded(true);
        });
    };
    const schedule = () => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(reload, RELOAD_DELAY_MS);
    };
    reload();
    const unlistens = RELOAD_EVENTS.map((name) => listen(name, schedule));
    return () => {
      alive = false;
      if (timer) clearTimeout(timer);
      unlistens.forEach((u) => {
        void u.then((fn) => fn());
      });
    };
  }, [enabled, surface]);

  const loadOlder = useCallback(() => {
    if (!enabled || !hasMore || !cursor.current || loadingOlder.current) return;
    loadingOlder.current = true;
    invoke<InboxViewPayload>('get_inbox_view', { filter: 'all', surface, cursor: cursor.current, limit: INBOX_PAGE })
      .then((view) => {
        setRows((prev) => {
          const seen = new Set(prev.map((r) => r.row_id));
          return [...prev, ...view.rows.filter((r) => !seen.has(r.row_id))];
        });
        setHasMore(view.has_more);
        cursor.current = view.cursor;
        seedCounts(view);
      })
      .catch((e) => Logger.debug('[Inbox] older page failed:', e))
      .finally(() => {
        loadingOlder.current = false;
      });
  }, [enabled, hasMore, surface]);

  const patch = useCallback((fn: (rows: InboxItem[]) => InboxItem[]) => setRows(fn), []);

  return { rows, hasMore, loaded, loadOlder, patch };
}
