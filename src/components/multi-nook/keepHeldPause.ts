/** How long after a pointer press or key press inside a tile a `play` event
 *  still counts as the viewer's own. */
export const VIEWER_PLAY_WINDOW_MS = 1000;

/** Whether a `play` event came from the viewer: a press landed in the tile just
 *  before it. `gestureAt` is 0 when the tile has seen no press. */
export function isViewerPlay(gestureAt: number, now: number): boolean {
  return gestureAt > 0 && now - gestureAt <= VIEWER_PLAY_WINDOW_MS;
}

/** Whether a `play` event on a tile must be undone to keep the viewer's pause:
 *  the tile holds a pause, its start point has not run yet, and the viewer did
 *  not press play. A play after the start point, or the viewer's own, ends the
 *  pause instead. */
export function shouldKeepHeldPause(state: {
  playStarted: boolean;
  held: boolean;
  viewerPlay: boolean;
}): boolean {
  return state.held && !state.playStarted && !state.viewerPlay;
}
