// Where a landscape brightness / volume swipe may start. The top and bottom
// edges belong to the system: the notification shade comes down from the top,
// the bars and the home swipe come up from the bottom, and immersive mode hands
// those touches to the app as well. Kept beside the gesture rather than in
// Rust: the answer is needed synchronously at pointerdown, and it is arithmetic
// on the finger's position (the iPhone's twin is SNKit `Dial.canStart`).

/** The quiet strip at each edge: about a fingertip, in CSS px (= dp). */
export const DIAL_EDGE_GUARD = 48;

/**
 * Whether a press `y` px down a screen `height` tall may turn a dial. Each
 * strip is a fingertip, or the system's own gesture strip when that is larger.
 */
export function dialCanStart(
  y: number,
  height: number,
  edgeTop: number,
  edgeBottom: number,
): boolean {
  const top = Math.max(DIAL_EDGE_GUARD, edgeTop);
  const bottom = Math.max(DIAL_EDGE_GUARD, edgeBottom);
  return y >= top && y <= height - bottom;
}
