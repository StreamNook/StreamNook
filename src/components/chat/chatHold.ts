// Two rules of the chat list's touch handling, kept pure so they run in tests.

/** How long a lifted press keeps its row held when no click follows it (a long press with no
 *  move, a lift outside the window). */
export const THAW_BACKSTOP_MS = 400;

/**
 * Called when the button comes up on a held row: the hold ends one tick after the press's own
 * click, which Chromium aims at the layout of the moment it lands, so the click finds the row it
 * came down on. With no click, it ends `backstopMs` after the button came up. Listens on `target`
 * (the window) in the capture phase, ahead of every handler that stops the click. Returns the
 * disarm, called when a new hold starts or the list lets go another way.
 */
export function armThaw(target: EventTarget, release: () => void, backstopMs = THAW_BACKSTOP_MS): () => void {
  let armed = true;
  let after: ReturnType<typeof setTimeout> | undefined;
  const onClick = () => {
    target.removeEventListener('click', onClick, true);
    clearTimeout(backstop);
    after = setTimeout(fire, 0);
  };
  const disarm = () => {
    armed = false;
    clearTimeout(backstop);
    clearTimeout(after);
    target.removeEventListener('click', onClick, true);
  };
  const fire = () => {
    if (!armed) return;
    disarm();
    release();
  };
  const backstop = setTimeout(fire, backstopMs);
  target.addEventListener('click', onClick, true);
  return disarm;
}

/**
 * What a touch move means for the list's pause: `pause` (the finger heads into older messages),
 * `clear` (back toward live) or `none`. Without `ratio` it is the list's own rule: any downward
 * move pauses. A host whose rows take sideways swipes passes the swipe's ratio while a swipe could
 * still start: then a downward move pauses only when it is vertical enough not to be that swipe
 * (`abs(dy) * ratio > abs(dx)`, measured from where the touch began).
 */
export function touchPauseIntent(
  fingerDelta: number,
  dxTotal: number,
  dyTotal: number,
  ratio?: number,
): 'pause' | 'clear' | 'none' {
  if (fingerDelta < 0) return 'clear';
  if (fingerDelta === 0) return 'none';
  if (ratio === undefined) return 'pause';
  return Math.abs(dyTotal) * ratio > Math.abs(dxTotal) ? 'pause' : 'none';
}
