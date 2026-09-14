// Haptics for touch gestures.
//
// The budget is spent on STATE CHANGES, never on motion. Buzzing on every frame
// of a drag, or on every snap while dialling a duration, reads as a broken phone
// rather than as feedback; you only want a tick when the thing under your finger
// actually became something else.
//
// `navigator.vibrate` needs no permission and is a no-op where unsupported, so
// every call is guarded and nothing here can throw.

function buzz(pattern: number | number[]): void {
  try {
    navigator.vibrate?.(pattern);
  } catch {
    /* unsupported or blocked; haptics are never load-bearing */
  }
}

/** A gesture became active (long-press armed). The heaviest single tick. */
export function hapticArm(): void {
  buzz(15);
}

/** The selection changed to a different target. Deliberately light. */
export function hapticTick(): void {
  buzz(8);
}

/** A continuous dial crossed a magnitude boundary (seconds -> minutes -> ...). */
export function hapticStep(): void {
  buzz(5);
}

/** An action was committed. */
export function hapticCommit(): void {
  buzz(20);
}

/** A destructive action was committed; distinct on purpose so a ban does not
 *  feel like a copy. */
export function hapticDestructive(): void {
  buzz([15, 40, 15]);
}
