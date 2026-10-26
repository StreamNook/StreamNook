/**
 * The <video> element's own routes into the player controller. Every way to
 * pause or play ends in one of two events on the element: a single click on
 * the picture asks the player to toggle, and a `pause` (the Plyr button,
 * picture-in-picture, media keys, the hotkey and the palette all end there)
 * is reported to the earning gate and handed to the pause park.
 */

/** A single click waits this long so a double-click can cancel it. */
export const CLICK_TOGGLE_DELAY_MS = 250;

/** Blur the focused text box, if one has focus: the player keys stand aside
 *  while you type (keybindings), so this hands them back. */
export function releaseTypingFocus(doc: Pick<Document, 'activeElement'> = document): void {
  const el = doc.activeElement as (HTMLElement & { isContentEditable?: boolean }) | null;
  if (!el || typeof el.blur !== 'function') return;
  const tag = el.tagName;
  if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || el.isContentEditable) el.blur();
}

export interface PauseRouteDeps {
  /** Toggle play and pause on the player. */
  togglePlay: () => void;
  /** Whether a click on the picture toggles play (the Click to pause
   *  setting), read at click time. Absent means it does. */
  clickToPause?: () => boolean;
  /** Move focus out of a text box, so the player keys work after a click on
   *  the picture whichever way Click to pause is set. */
  releaseFocus?: () => void;
  /** Tell the earning gate the video paused. */
  reportPaused: () => void;
  /** Hand the pause to the pause park. */
  parkPaused: () => void;
}

type PauseRouteTarget = Pick<HTMLVideoElement, 'addEventListener' | 'removeEventListener'>;

/**
 * Wire the element. A click first moves focus out of a text box. With Click to
 * pause on, a single click then toggles play after a short hold; a double
 * click only cancels that hold, so it never blips play/pause (which on a live
 * stream forces a rebuffer). Fullscreen on a double click belongs to Plyr,
 * which owns it on its container. Every `pause`, however it came, reports and
 * parks, so turning the click off never keeps a paused stream downloading.
 * Returns the detach.
 */
export function attachPauseRoutes(video: PauseRouteTarget, deps: PauseRouteDeps): () => void {
  let clickTimer: ReturnType<typeof setTimeout> | null = null;
  const onClick = () => {
    deps.releaseFocus?.();
    if (deps.clickToPause && !deps.clickToPause()) return;
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
