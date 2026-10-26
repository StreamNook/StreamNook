/**
 * The <video> element's own routes into the player controller. Every way to
 * pause or play ends in one of two events on the element: a single click on
 * the picture asks the player to toggle, and a `pause` (the Plyr button,
 * picture-in-picture, media keys, the hotkey and the palette all end there)
 * is reported to the earning gate and handed to the pause park.
 */

/** A single click waits this long so a double-click can cancel it. */
export const CLICK_TOGGLE_DELAY_MS = 250;

export interface PauseRouteDeps {
  /** Toggle play and pause on the player. */
  togglePlay: () => void;
  /** Tell the earning gate the video paused. */
  reportPaused: () => void;
  /** Hand the pause to the pause park. */
  parkPaused: () => void;
}

type PauseRouteTarget = Pick<HTMLVideoElement, 'addEventListener' | 'removeEventListener'>;

/**
 * Wire the element. A single click toggles play after a short hold; a double
 * click only cancels that hold, so it never blips play/pause (which on a live
 * stream forces a rebuffer). Fullscreen on a double click belongs to Plyr,
 * which owns it on its container. Every `pause` reports and parks. Returns
 * the detach.
 */
export function attachPauseRoutes(video: PauseRouteTarget, deps: PauseRouteDeps): () => void {
  let clickTimer: ReturnType<typeof setTimeout> | null = null;
  const onClick = () => {
    if (clickTimer) return; // second click of a pair: the double-click handler cancels it
    clickTimer = setTimeout(() => {
      clickTimer = null;
      deps.togglePlay();
    }, CLICK_TOGGLE_DELAY_MS);
  };
  const onDblClick = () => {
    if (clickTimer) {
      clearTimeout(clickTimer);
      clickTimer = null;
    }
  };
  const onPause = () => {
    deps.reportPaused();
    deps.parkPaused();
  };
  video.addEventListener('click', onClick);
  video.addEventListener('dblclick', onDblClick);
  video.addEventListener('pause', onPause);
  return () => {
    if (clickTimer) clearTimeout(clickTimer);
    clickTimer = null;
    video.removeEventListener('click', onClick);
    video.removeEventListener('dblclick', onDblClick);
    video.removeEventListener('pause', onPause);
  };
}
