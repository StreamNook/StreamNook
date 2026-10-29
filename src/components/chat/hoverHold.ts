// The moderator's hover hold (ChatMessageList): while a moderator aims at a
// mod action the chat list holds still, so a fast chat cannot slide the
// target away. The rules live here, apart from the DOM, so they are tested
// without a browser; the list feeds in what the pointer did.

/** How far the pointer must move, x plus y, from its last counted spot for a
 *  move to count: content sliding under a still pointer repeats its spot, and
 *  a hand settling on the mouse shifts it a pixel or two. The system's own
 *  click jitter is the same size. */
export const AIM_MOVE_PX = 4;

/** A quick move, which aims again after rows slid under a still pointer: at
 *  least this far, x plus y, from where the pointer was when the last
 *  AIM_QUICK_MS began. Drift and tremor never get there; any real reach for
 *  a button does, even in one jump from rest. */
export const AIM_QUICK_PX = 12;
export const AIM_QUICK_MS = 250;

/** How long the pointer can be off every hold zone before the hold lets go:
 *  long enough to cross the gap between one line's buttons and the next,
 *  short enough that leaving them feels immediate. */
export const HOLD_GRACE_MS = 250;

/** How long a pointer that stopped keeps the hold: time to read a line and
 *  decide, short enough that a pointer left resting on chat lets it run live
 *  again. The same wherever it stopped: the hover dock covers half of every
 *  row and the slider grip runs down the list's edge, so a resting pointer is
 *  often on one. */
export const HOLD_REST_MS = 3000;

/** How long the hold must last before its pill may show, so a pointer
 *  crossing chat never flashes it. */
export const HOLD_PILL_DELAY_MS = 1000;

/** The pill shows only with at least this long left before the hold lets go,
 *  so it never appears just to vanish. */
export const HOLD_PILL_MIN_MS = 800;

/** How often a pill that is not due yet looks again. */
export const HOLD_PILL_RECHECK_MS = 250;

/** How close the pointer may come to the pill before it steps aside, so it
 *  never covers what the pointer aims at. */
export const HOLD_PILL_CLEAR_PX = 32;

/** Whether a pointer at (x, y) on screen moved far enough from the last
 *  counted spot to count (AIM_MOVE_PX). The first move always counts. Callers
 *  move `last` only on a move that counts, so slow creeping still adds up. */
export function movedEnough(last: { x: number; y: number } | null, x: number, y: number): boolean {
  return last === null || Math.abs(x - last.x) + Math.abs(y - last.y) >= AIM_MOVE_PX;
}

export interface MoveTrail {
  /** The pointer is at (x, y) on screen now. */
  add(x: number, y: number): void;
  /** Whether it covered AIM_QUICK_PX within the last AIM_QUICK_MS. */
  quick(): boolean;
  /** Forget the way here (rows moved under it, so no move before now
   *  counts), but keep where the pointer is, so one reach from there does. */
  clear(): void;
}

export function createMoveTrail(): MoveTrail {
  const points: { x: number; y: number; t: number }[] = [];
  return {
    add(x, y) {
      const t = Date.now();
      points.push({ x, y, t });
      // Keep one point from before the window, where the pointer was when the
      // window began, so a single quick jump from rest counts.
      while (points.length > 1 && t - points[1].t > AIM_QUICK_MS) points.shift();
    },
    quick() {
      const last = points[points.length - 1];
      if (!last) return false;
      return points.some((p) => Math.abs(last.x - p.x) + Math.abs(last.y - p.y) >= AIM_QUICK_PX);
    },
    clear() {
      points.splice(0, points.length - 1);
    },
  };
}

export interface HoverHold {
  /** A counted move on a hold zone: hold, and start the rest over. */
  move(): void;
  /** A one-press mod button went down: hold, with no rest until it comes up,
   *  so its row cannot slide away between press and release. */
  pressDown(): void;
  /** The button came up: the rest starts over, or the grace if the pointer
   *  left the zones meanwhile. */
  pressUp(): void;
  /** The pointer went from a hold zone to a spot that is none. */
  leave(): void;
  /** Let go now; the next counted move on a zone holds again (a wheel
   *  scroll, another feed). */
  drop(): void;
  /** How long until the hold lets go if nothing else happens, counting a
   *  leave in progress; 0 when nothing is held. */
  restLeft(): number;
  /** How long the current hold has lasted; 0 when nothing is held. */
  heldFor(): number;
  /** Let go and stop every timer (unmount). */
  dispose(): void;
}

export function createHoverHold(onChange: (held: boolean) => void): HoverHold {
  let held = false;
  let heldAt = 0;
  let pressing = false;
  // The pointer left the zones while a button was down.
  let outside = false;
  let grace: ReturnType<typeof setTimeout> | undefined;
  let graceAt: number | null = null;
  let rest: ReturnType<typeof setTimeout> | undefined;
  let restAt = 0;
  const stopTimers = () => {
    clearTimeout(grace);
    clearTimeout(rest);
    grace = undefined;
    rest = undefined;
    graceAt = null;
  };
  const release = () => {
    stopTimers();
    pressing = false;
    outside = false;
    if (!held) return;
    held = false;
    onChange(false);
  };
  const begin = () => {
    if (held) return;
    held = true;
    heldAt = Date.now();
    onChange(true);
  };
  const armRest = () => {
    restAt = Date.now() + HOLD_REST_MS;
    rest = setTimeout(release, HOLD_REST_MS);
  };
  const armGrace = () => {
    clearTimeout(grace);
    graceAt = Date.now() + HOLD_GRACE_MS;
    grace = setTimeout(release, HOLD_GRACE_MS);
  };
  return {
    move() {
      stopTimers();
      outside = false;
      if (!pressing) armRest();
      begin();
    },
    pressDown() {
      stopTimers();
      pressing = true;
      outside = false;
      begin();
    },
    pressUp() {
      if (!pressing) return;
      pressing = false;
      if (!held) return;
      stopTimers();
      if (outside) armGrace();
      else armRest();
      outside = false;
    },
    leave() {
      if (!held) return;
      if (pressing) {
        outside = true;
        return;
      }
      armGrace();
    },
    drop: release,
    restLeft() {
      if (!held) return 0;
      if (pressing) return HOLD_REST_MS;
      const end = graceAt === null ? restAt : Math.min(restAt, graceAt);
      return Math.max(0, end - Date.now());
    },
    heldFor() {
      return held ? Date.now() - heldAt : 0;
    },
    dispose: release,
  };
}

/**
 * Calls `show` once the hold's pill is due: the hold has lasted
 * HOLD_PILL_DELAY_MS and has HOLD_PILL_MIN_MS or more left, looking again
 * every HOLD_PILL_RECHECK_MS until then. Returns the cancel.
 */
export function whenPillDue(heldFor: () => number, restLeft: () => number, show: () => void): () => void {
  let timer = setTimeout(function check() {
    if (heldFor() >= HOLD_PILL_DELAY_MS && restLeft() >= HOLD_PILL_MIN_MS) show();
    else timer = setTimeout(check, HOLD_PILL_RECHECK_MS);
  }, Math.max(0, HOLD_PILL_DELAY_MS - heldFor()));
  return () => clearTimeout(timer);
}

/** Whether `point` is within `px` of `box`. No point is never near. */
export function isNear(
  box: { left: number; top: number; right: number; bottom: number },
  point: { x: number; y: number } | null,
  px: number,
): boolean {
  if (point === null) return false;
  return point.x >= box.left - px && point.x <= box.right + px && point.y >= box.top - px && point.y <= box.bottom + px;
}

/**
 * How many rows the live feed holds past the held view's newest row. Rows are
 * matched by id first (Rust restamps a row as a new object with the same id),
 * then by identity. When that row has left the live buffer, more arrived than
 * the buffer keeps: one more than everything in it.
 */
export function rowsSinceHold<T>(held: readonly T[], live: readonly T[], idOf: (row: T) => string | null): number {
  if (held.length === 0) return live.length;
  const tail = held[held.length - 1];
  const id = idOf(tail);
  for (let i = live.length - 1; i >= 0; i--) {
    const row = live[i];
    if (row === tail || (id !== null && idOf(row) === id)) return live.length - 1 - i;
  }
  return live.length + 1;
}
