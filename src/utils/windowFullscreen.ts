import { Logger } from './logger';

// Win32 quirk: a borderless window (decorations: false) that is WS_MAXIMIZE
// keeps its maximized chrome/taskbar visible even after setFullscreen(true).
// Track whether the window was maximized going in so we can restore it on exit.
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
      restoreMaximizedAfterFullscreen = await win.isMaximized();
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
