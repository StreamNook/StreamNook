import { invoke } from '@tauri-apps/api/core';
import { Logger } from './logger';
import type Plyr from 'plyr';
import { barClickExitsPlayerFullscreen } from './titleBarTuck';

/**
 * Put the window into full screen or take it out (no argument toggles), and
 * resolve to whether it is full screen afterwards.
 *
 * Rust owns the trip (`set_window_fullscreen`, commands/window_state.rs) for
 * both of the app's full screens, so the window comes back maximized if it
 * went in maximized whichever full screen it leaves from. A borderless window
 * that is maximized keeps the taskbar over it even in full screen, so Rust
 * restores it on the way in and maximizes it again on the way out (never on
 * macOS, where AppKit owns an animated transition that refuses a zoom).
 */
/** Plyr's tooltip wording for its full screen button, in the app's words
 *  ("full screen", the same as the title bar and Settings). Merged into every
 *  player's Plyr options. */
export const PLYR_FULLSCREEN_I18N = { enterFullscreen: 'Full screen', exitFullscreen: 'Exit full screen' };

export const setWindowFullscreen = (fullscreen?: boolean): Promise<boolean> =>
  invoke<boolean>('set_window_fullscreen', { fullscreen: fullscreen ?? null });

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
    // A fresh full screen starts as video only; chat is brought up on demand.
    // A player taking over an existing full screen (a raid, a quality change)
    // keeps whatever the viewer had.
    const fresh = entering && !useAppStore.getState().isPlayerFullscreen;
    useAppStore.setState({
      isPlayerFullscreen: entering,
      playerOverlayVisible: true,
      ...(fresh || !entering ? { fsChatOpen: false } : {}),
    });
  } catch {
    /* store not ready: nothing to overlay yet */
  }
  try {
    if (!entering) {
      // If the app is in its own borderless full-screen mode, the player only
      // borrowed an already-fullscreen window. Leave the window fullscreen on
      // exit so closing the video doesn't kick the whole app back to windowed.
      const { useAppStore } = await import('../stores/AppStore');
      if (useAppStore.getState().isWindowFullscreen) return;
    }
    // A player taking over from the previous one enters on a window that is
    // already full screen. Rust leaves such a window alone, so the maximize
    // remembered on the first entry survives the hand-over.
    await setWindowFullscreen(entering);
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

/**
 * Leave the player's own full screen through the player, so its exitfullscreen
 * event runs the usual bridge (syncTauriWindowFullscreen) and the window and
 * Plyr stay in step. Plyr keeps its instance on the media element.
 */
export const exitPlayerFullscreen = (): void => {
  const media = document.querySelector<HTMLMediaElement & { plyr?: Plyr }>(
    '.plyr--fullscreen-fallback video, .plyr--fullscreen-fallback audio',
  );
  const player = media?.plyr;
  if (player?.fullscreen?.active) {
    player.fullscreen.exit();
    return;
  }
  // Mid hand-off (the old player gone, the next not built yet): nothing to
  // exit through, so end the hand-off and the window's full screen directly.
  if (fullscreenHandoff) {
    clearTimeout(fullscreenHandoff);
    fullscreenHandoff = null;
  }
  void syncTauriWindowFullscreen(false);
};

const ACTIONABLE = 'button, a[href], [role="button"], [role="menuitem"], [role="menuitemradio"], [role="option"], [role="tab"], [role="link"]';

/** For a click inside the title bar or a menu it opened, while the player is
 *  full screen: leaves full screen first when the click changes the page. */
export const exitPlayerFullscreenForBarAction = (target: EventTarget | null): void => {
  if (!(target instanceof Element)) return;
  const control = target.closest(ACTIONABLE);
  const exits = barClickExitsPlayerFullscreen({
    playerFullscreen: true,
    actionable: !!control,
    opensMenu: !!control && (control.hasAttribute('aria-expanded') || control.hasAttribute('aria-haspopup')),
    keepsFullscreen: !!target.closest('[data-keeps-player-fullscreen]'),
  });
  if (exits) exitPlayerFullscreen();
};
