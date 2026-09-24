import { invoke } from '@tauri-apps/api/core';
import { create } from 'zustand';
import type { ActivityEvent } from '../types/activity';
import { Logger } from '../utils/logger';

// The MultiChat Activity feed as this window shows it. Kept separate from the
// main AppStore so nothing here can affect normal chat.
//
// The history itself is Rust's (services/activity_history_service.rs): one
// store for every window, persisted, capped per source so one busy channel
// can't evict another's history. This store holds only the events the open
// feed is showing, and drops whatever Rust's caps evict.

/** Where each window used to keep its own copy, moved into Rust once. */
const LEGACY_STORAGE_KEY = 'sn-activity-history-v1';

interface Appended {
  added: boolean;
  evicted: string[];
}

/** Hand a window's pre-Rust localStorage history to Rust, then forget it. */
async function importLegacyHistory(): Promise<void> {
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(LEGACY_STORAGE_KEY);
  } catch {
    return;
  }
  if (!raw) return;
  try {
    const parsed: unknown = JSON.parse(raw);
    if (Array.isArray(parsed) && parsed.length > 0) {
      await invoke('activity_import', { events: parsed });
    }
    localStorage.removeItem(LEGACY_STORAGE_KEY);
  } catch (err) {
    Logger.warn('[Activity] could not move saved history into the app store:', err);
  }
}

interface ActivityState {
  events: ActivityEvent[];
  addEvent: (event: ActivityEvent) => void;
  /** Purge the stored history for the given composite source keys (in-memory + disk). */
  purgeChannels: (sourceKeys: string[]) => void;
  /** Purge ALL stored activity. */
  clear: () => void;
  /** Drop the in-memory events to free RAM when the panel closes, WITHOUT touching
   *  the persisted copy. `hydrate()` restores them on reopen. */
  release: () => void;
  /** Reload events from the persisted store (on panel open, after a release). */
  hydrate: () => void;
}

export const useActivityStore = create<ActivityState>((set) => ({
  events: [],
  addEvent: (event) => {
    // Sources can echo the same event; the id check keeps the feed from
    // flashing a duplicate before Rust says so.
    let shown = false;
    set((state) => {
      if (event.id && state.events.some((e) => e.id === event.id)) return state;
      shown = true;
      return { events: [event, ...state.events] };
    });
    if (!shown) return;
    invoke<Appended>('activity_append', { event })
      .then(({ evicted }) => {
        if (evicted.length === 0) return;
        const gone = new Set(evicted);
        set((state) => ({ events: state.events.filter((e) => !gone.has(e.id)) }));
      })
      .catch((err) => Logger.warn('[Activity] failed to record event:', err));
  },
  purgeChannels: (sourceKeys) => {
    const drop = new Set(sourceKeys.map((k) => k.toLowerCase()));
    set((state) => ({ events: state.events.filter((e) => !drop.has(e.channel.toLowerCase())) }));
    invoke('activity_purge', { sourceKeys }).catch((err) =>
      Logger.warn('[Activity] failed to purge history:', err),
    );
  },
  clear: () => {
    set({ events: [] });
    invoke('activity_clear').catch((err) => Logger.warn('[Activity] failed to clear history:', err));
  },
  release: () => set({ events: [] }),
  hydrate: () => {
    void (async () => {
      await importLegacyHistory();
      try {
        const stored = await invoke<ActivityEvent[]>('activity_load');
        // Anything that arrived while the load was in flight is newer than the
        // snapshot and already on its way to Rust; keep it on top.
        set((state) => {
          const storedIds = new Set(stored.map((e) => e.id));
          const arrived = state.events.filter((e) => !storedIds.has(e.id));
          return { events: [...arrived, ...stored] };
        });
      } catch (err) {
        Logger.warn('[Activity] failed to load history:', err);
      }
    })();
  },
}));
