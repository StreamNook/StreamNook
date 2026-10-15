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
// Rust owns the throttle (`note_app_resumed`), sharing one freshness stamp with
// the first load and the pull, so the iPhone app follows the same rule.
import { invoke } from '@tauri-apps/api/core';
import { Logger } from '../utils/logger';

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

/**
 * Reload the followed list if it has gone stale. Safe to call on every resume.
 *
 * Rust reloads it when nobody asked for it for a minute, then hype trains and
 * collaborations, since a refreshed list with stale badges is its own kind of
 * wrong; the lists land as `home-snapshot` events the store applies. Signed
 * out, Rust does nothing.
 */
export async function refreshFollowingIfStale(): Promise<void> {
  try {
    if (await invoke<boolean>('note_app_resumed')) bumpPreviewStamp();
  } catch (err) {
    Logger.warn('[Following] resume refresh failed:', err);
  }
}
