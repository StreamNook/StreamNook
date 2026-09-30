import { Logger } from './logger';
import { IS_MAC } from './platform';
import type Plyr from 'plyr';

// Win32 quirk: a borderless window (decorations: false) that is WS_MAXIMIZE
// keeps its maximized chrome/taskbar visible even after setFullscreen(true).
// Track whether the window was maximized going in so we can restore it on exit.
//
// macOS never takes this path. There the window is decorated and
// `toggleFullScreen:` is an animated transition AppKit owns: zooming the
// window right before or right after it fights that transition, and when
// AppKit refuses the entry the window keeps reporting itself full screen
// while it is not, so every later toggle does nothing until the green
// button is used.
let restoreMaximizedAfterFullscreen = false;

/**
 * Promote (or demote) the Tauri window to true OS fullscreen in lockstep with
 * Plyr's CSS fullscreen.
 *
 * Plyr is forced into CSS-only fullscreen (fallback: 'force') because the window
 * is borderless, so HTML5 element-fullscreen would only scope to the window
 * viewport and never cover the taskbar. Bridging Plyr's enterfullscreen /
 * exitfullscreen events to this keeps the real OS window covering the whole
 * screen. Both the single player and MultiNook tiles share this bridge.
 */
export const syncTauriWindowFullscreen = async (entering: boolean): Promise<void> => {
  // Flag first, before any await: the chat overlay keys off this and must
  // flip in the same frame Plyr swaps its fullscreen class.
  try {
    const { useAppStore } = await import('../stores/AppStore');
    useAppStore.setState({ isPlayerFullscreen: entering, playerOverlayVisible: true });
  } catch {
    /* store not ready: nothing to overlay yet */
  }
  try {
    const { getCurrentWindow, currentMonitor, PhysicalPosition } = await import('@tauri-apps/api/window');
    const win = getCurrentWindow();
    if (entering) {
      // A player taking over from the previous one enters on a window that is
      // already full screen; the maximize bookkeeping from the first entry
      // has to survive it.
      if (await win.isFullscreen()) return;
      restoreMaximizedAfterFullscreen = !IS_MAC && (await win.isMaximized());
      if (restoreMaximizedAfterFullscreen) {
        await win.unmaximize();
      }
      await win.setFullscreen(true);
    } else {
      // If the app is in its own borderless full-screen mode, the player only
      // borrowed an already-fullscreen window. Leave the window fullscreen on
      // exit so closing the video doesn't kick the whole app back to windowed.
      const { useAppStore } = await import('../stores/AppStore');
      if (useAppStore.getState().isWindowFullscreen) return;
      await win.setFullscreen(false);
      if (restoreMaximizedAfterFullscreen) {
        // After repeated fullscreen→exit cycles, Win32's saved restore
        // placement can drift, leaving the next maximize() bound to the
        // wrong rect (window ends up partially off-screen). Anchor to the
        // current monitor's origin first so maximize() snaps to its work area.
        const monitor = await currentMonitor();
        if (monitor) {
          await win.setPosition(new PhysicalPosition(monitor.position.x, monitor.position.y));
        }
        await win.maximize();
        restoreMaximizedAfterFullscreen = false;
      }
    }
  } catch (err) {
    Logger.error('[Fullscreen] Failed to sync Tauri window:', err);
  }
};

// Full screen outlives the player that entered it. Plyr.destroy() neither
// leaves full screen nor fires exitfullscreen, and a stream switch (a raid,
// an auto-switch, a quality change, a platform change) destroys the player and
// builds another. Without a hand-off the store kept isPlayerFullscreen set
// with no player in full screen: chat stayed lifted over the video as the
// full-screen column and nothing on screen could exit it. This lives in the
// page because Plyr does. The dying player hands full screen over, the next
// player claims it, and if none is built in time full screen ends.
const FULLSCREEN_HANDOFF_MS = 4000;
let fullscreenHandoff: ReturnType<typeof setTimeout> | null = null;

/** Call before destroying a Plyr instance. `nextPlayerComing` is false when
 *  the stream stopped or went offline, which ends full screen at once. */
export const handOffPlayerFullscreen = (
  player: Plyr | null | undefined,
  { nextPlayerComing }: { nextPlayerComing: boolean },
): void => {
  if (!player?.fullscreen?.active) return;
  if (fullscreenHandoff) clearTimeout(fullscreenHandoff);
  fullscreenHandoff = null;
  if (!nextPlayerComing) {
    void syncTauriWindowFullscreen(false);
    return;
  }
  fullscreenHandoff = setTimeout(() => {
    fullscreenHandoff = null;
    void syncTauriWindowFullscreen(false);
  }, FULLSCREEN_HANDOFF_MS);
};

/** Call once a new Plyr instance has its fullscreen listeners bound. */
export const claimPlayerFullscreen = (player: Plyr): void => {
  if (!fullscreenHandoff) return;
  clearTimeout(fullscreenHandoff);
  fullscreenHandoff = null;
  player.fullscreen.enter();
};
