// When a moderator's one-press buttons may act (ChatMessageList). Chat never
// pauses by itself, so in a live chat rows slide under the pointer; a press
// there would land on a chatter nobody aimed at. The list samples the rows
// under the pointer after every paint, and these rules say what counts as
// rows moving and when they have held still long enough.

/** How long rows must hold still under the pointer before a mod button there
 *  acts. A press decided on the rows that were there before lands within about
 *  150 to 200 ms; seeing a change and reacting to it takes about 250 ms. */
export const STILL_MS = 350;

/** A tap this soon after one at the same spot is the same gesture. */
export const TAP_AGAIN_MS = 500;
export const TAP_AGAIN_PX = 16;

/** A row's edges on screen. */
export interface Edges {
  top: number;
  bottom: number;
}

/** A row moved when both its edges moved. A row that grew or shrank in place
 *  keeps one edge: its own line buttons opening, an emote loading in it. */
export function edgesMoved(before: Edges, after: Edges): boolean {
  return Math.abs(after.top - before.top) > 0.5 && Math.abs(after.bottom - before.bottom) > 0.5;
}

/** The rows have held still long enough for a press to act. */
export function isStill(movedAt: number | null, now: number): boolean {
  return movedAt === null || now - movedAt >= STILL_MS;
}

/** Where a mod button sits on screen. */
export interface Box {
  top: number;
  right: number;
  bottom: number;
  left: number;
}

/** The same place on screen, each edge within half a pixel. A button that
 *  re-renders in place (Ban waiting for its second press) is the same button. */
export function sameBox(a: Box | null, b: Box | null): boolean {
  if (!a || !b) return a === b;
  return (
    Math.abs(a.top - b.top) <= 0.5 &&
    Math.abs(a.right - b.right) <= 0.5 &&
    Math.abs(a.bottom - b.bottom) <= 0.5 &&
    Math.abs(a.left - b.left) <= 0.5
  );
}

/** A touch coming down: where, when, and the mod button under it (null for
 *  none). */
export interface Tap {
  x: number;
  y: number;
  t: number;
  box: Box | null;
}

/** A tap that repeats the one before at the same spot, on a mod button that
 *  tap did not come down on: a line's buttons open under a tap on its text,
 *  so a quick second tap there is the same gesture, not an aim. */
export function tapAgain(down: Tap | null, last: Tap | null): boolean {
  return (
    down !== null &&
    last !== null &&
    down.box !== null &&
    down.t - last.t < TAP_AGAIN_MS &&
    Math.abs(down.x - last.x) + Math.abs(down.y - last.y) <= TAP_AGAIN_PX &&
    !sameBox(last.box, down.box)
  );
}
