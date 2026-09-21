// Freshness of the followed-streams list for the mobile shell.
//
// Why this exists at all: the Android activity SURVIVES backgrounding, and it
// survives closing a PiP window too (verified against dumpsys - the process and
// the ActivityRecord both stay). So the zustand store keeps whatever it last
// fetched, and reopening the app can show live/offline state, viewer counts and
// titles from hours ago.
//
// Desktop never needed this: Sidebar reloads on hover and on expand, and Home
// reloads on mount, so ordinary navigation keeps the list fresh incidentally.
// The phone shell has no equivalent trigger - FollowingScreen's mount effect is
// guarded on the list being EMPTY - so before this the only refresh in the whole
// app was pull-to-refresh.
//
// Throttled rather than unconditional because resume fires constantly on a
// phone: every app switch, every return from PiP, every notification glance.
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../stores/AppStore';
import { Logger } from '../utils/logger';

const STALE_AFTER_MS = 60_000;

let lastLoadedAt = 0;

// The preview-image generation. Twitch serves a live preview at ONE fixed URL
// per channel and size and swaps the frame behind it every few minutes, so a
// card whose URL never changes keeps whatever the WebView cached: a streamer
// could switch category and the card would still show the old game. Appended
// to the preview URL as a query string, this changes only when a list is
// refreshed (pull, resume), which is exactly when a fresh frame is wanted,
// and stays put across ordinary re-renders so nothing refetches for free.
let previewGeneration = Date.now();

export function previewStamp(): number {
  return previewGeneration;
}

/** New previews next paint. Every explicit list refresh calls this. */
export function bumpPreviewStamp(): void {
  previewGeneration = Date.now();
}

/** Record that the list was just fetched, so a resume moments later is a no-op.
 *  Called by every path that loads it, not just this module's. */
export function markFollowingFresh(): void {
  lastLoadedAt = Date.now();
  bumpPreviewStamp();
}

/**
 * Reload the followed list if it has gone stale. Safe to call on every resume.
 *
 * Hype-train statuses ride along because they are what the row badges render
 * from, and a refreshed list with stale badges is its own kind of wrong.
 */
export async function refreshFollowingIfStale(maxAgeMs = STALE_AFTER_MS): Promise<void> {
  const store = useAppStore.getState();
  if (!store.isAuthenticated) return;
  if (Date.now() - lastLoadedAt < maxAgeMs) return;
  // Stamp up front: a slow request should not let a second resume start another.
  markFollowingFresh();
  try {
    await store.loadFollowedStreams();
    // Hype-train statuses are a section of the Rust-owned Home snapshot now
    // (services::home_snapshot); the result lands as a `home-snapshot` event
    // the store applies. Floored at 15 s per section on the Rust side.
    await invoke('refresh_home_section', { section: 'hype_trains' });
  } catch (err) {
    Logger.warn('[Following] resume refresh failed:', err);
  }
}
