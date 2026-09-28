// Publishes whether the window is maximized or full screen to CSS, for the
// Linux rounded frame (html[data-window-frame="rounded"] in globals.css). A
// maximized or full-screen window is square, as on Windows, so the rounding and
// the hairline border come off. Only started by main.tsx when this page draws
// its own frame.

import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';

/** What `get_window_flags` answers (commands/window_state.rs): the three
 *  window questions the Linux frame code asks together, in one round trip
 *  rather than one plugin call each. */
export interface WindowFlags {
  resizable: boolean;
  maximized: boolean;
  fullscreen: boolean;
}

export function trackWindowFrameState(): void {
  const win = getCurrentWindow();
  const root = document.documentElement;
  const refresh = () => {
    invoke<WindowFlags>('get_window_flags')
      .then(({ maximized, fullscreen }) => {
        root.dataset.windowState = fullscreen ? 'fullscreen' : maximized ? 'maximized' : 'normal';
      })
      .catch(() => {});
  };
  refresh();
  void win.onResized(refresh);
}
