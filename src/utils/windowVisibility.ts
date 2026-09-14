// Whether THIS window is hidden, as Rust knows it.
//
// Chromium cannot see that the window is minimized here (native occlusion
// detection is off, see the browser args in main.rs), so `document.hidden`
// stays false while the window sits in the taskbar and every gate that read
// it kept running. Rust polls `is_minimized` and emits `window-visibility` to
// the window; this module folds that into one `isWindowHidden()` that the
// existing gates read instead of `document.hidden`, marks `<html>` with
// `data-sn-hidden` for the stylesheet (animations paused, video element out of
// the render tree, audio unaffected) and fires a `sn-visibility` DOM event so
// interval hooks can catch up the moment the window is back.
import { listen } from '@tauri-apps/api/event';

let rustHidden = false;
let initialised = false;

function applyHidden(next: boolean) {
  if (rustHidden === next) return;
  rustHidden = next;
  try {
    if (next) document.documentElement.dataset.snHidden = 'true';
    else delete document.documentElement.dataset.snHidden;
    window.dispatchEvent(new Event('sn-visibility'));
  } catch {
    /* non-DOM context */
  }
}

/** Register the Rust listener once per window. Safe to call repeatedly. */
export function initWindowVisibility(): void {
  if (initialised || typeof window === 'undefined') return;
  initialised = true;
  void listen<{ hidden: boolean }>('window-visibility', (event) => {
    applyHidden(!!event.payload?.hidden);
  }).catch(() => {
    initialised = false;
  });
}

/** True when the page is hidden by the browser's own account OR minimized
 *  per Rust. Use this instead of `document.hidden`. */
export function isWindowHidden(): boolean {
  if (typeof document !== 'undefined' && document.visibilityState !== 'visible') return true;
  return rustHidden;
}

/** Subscribe to visibility changes from either source. Returns the unsubscribe. */
export function onWindowVisibility(cb: () => void): () => void {
  if (typeof window === 'undefined') return () => {};
  document.addEventListener('visibilitychange', cb);
  window.addEventListener('sn-visibility', cb);
  return () => {
    document.removeEventListener('visibilitychange', cb);
    window.removeEventListener('sn-visibility', cb);
  };
}

initWindowVisibility();
