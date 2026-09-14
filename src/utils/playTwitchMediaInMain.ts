// Route Twitch clip/VOD playback to the main window's player.
//
// Every Tauri WebView has its own JS context and its own AppStore instance, so
// calling `useAppStore.getState().playMedia(...)` inside a MultiChat popout
// plays into the popout's own store — but a chat-only popout has no video
// player, so nothing visible happens. These helpers detect a popout and emit a
// Tauri event (popout → main); the main-window listener in
// `utils/multichatTrayBridge.ts` shows/focuses main and plays there. In the main
// window we just call playMedia directly.
//
// Caller contract: callers don't need to know whether they're in a popout. They
// call the helper and the clip/VOD opens in the app's player wherever it lives.

import { useAppStore, type MediaInfo } from '../stores/AppStore';
import { Logger } from './logger';
import { openExternal } from './openExternal';

function isPopoutWindow(): boolean {
  if (typeof window === 'undefined') return false;
  const hash = window.location.hash;
  return hash.startsWith('#/multichat') || hash.startsWith('#/profile');
}

// Shared helper, not a local copy. This is the browser fallback for a clip/VOD
// that will not play, and it is reached from LinkPreviewCard - so on Android it
// was a dead end on top of a dead end.

export async function playTwitchMediaInMain(
  type: 'clip' | 'video',
  url: string,
  info: MediaInfo,
): Promise<void> {
  if (isPopoutWindow()) {
    try {
      // Going live may have closed main; ensure it's back + listening before we
      // emit (fast show+focus if it was only hidden). Phase 2 will play VODs in
      // the popout itself; until then both clip + VOD route to main's player.
      const { ensureMainAndEmit } = await import('./ensureMainWindow');
      await ensureMainAndEmit('play-twitch-media-in-main', { type, url, info });
      return;
    } catch (err) {
      // Bridge unavailable — fall back to the browser so the link is never dead.
      Logger.error('[playTwitchMediaInMain] emit failed, opening externally:', err);
      await openExternal(url);
      return;
    }
  }

  try {
    await useAppStore.getState().playMedia(type, url, info);
  } catch (err) {
    Logger.error('[playTwitchMediaInMain] playMedia failed, opening externally:', err);
    await openExternal(url);
  }
}
