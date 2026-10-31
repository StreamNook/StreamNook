// Whether a touch at the top of a list is a pull to refresh. Kept apart from
// PullToRefresh so the rule is tested on its own.

/** A finger's first few pixels say nothing about its direction. */
export const PULL_SLOP_PX = 10;

/** `wait` inside the slop (neither claim nor hand back), `claim` for a pull that
 *  is more down than sideways at the list's top, `release` for anything else. A
 *  move the browser can no longer cancel is already its own scroll (a card's
 *  sideways tag line, say), so it is released too. */
export function pullDecision(
  dx: number,
  dy: number,
  scrollTop: number,
  cancelable: boolean,
): 'wait' | 'claim' | 'release' {
  if (Math.abs(dx) < PULL_SLOP_PX && Math.abs(dy) < PULL_SLOP_PX) return 'wait';
  if (dy <= 0 || dy <= Math.abs(dx) || scrollTop > 0 || !cancelable) return 'release';
  return 'claim';
}
