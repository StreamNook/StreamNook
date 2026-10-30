// Haptics for touch gestures.
//
// The budget is spent on STATE CHANGES, never on motion. Buzzing on every frame
// of a drag, or on every snap while dialing a duration, reads as a broken phone
// rather than as feedback; you only want a tick when the thing under your finger
// actually became something else.
//
// On Android the shell's own touch feedback goes first (MainActivity `haptic`):
// the WebView's `navigator.vibrate` needs the VIBRATE permission, which the app
// does not hold, so it does nothing there, while the shell's needs no
// permission and follows the phone's touch-feedback switch. `navigator.vibrate`
// stays as the fallback wherever there is no shell. Every call is guarded, so
// nothing here can throw.

type HapticKind = 'arm' | 'tick' | 'step' | 'commit' | 'destructive';

function buzz(pattern: number | number[]): void {
  try {
    navigator.vibrate?.(pattern);
  } catch {
    /* unsupported or blocked; haptics are never load-bearing */
  }
}

function pulse(kind: HapticKind, pattern: number | number[]): void {
  try {
    const shell = (window as Window & { SNInsets?: { haptic?(kind: string): void } }).SNInsets;
    if (typeof shell?.haptic === 'function') {
      shell.haptic(kind);
      return;
    }
  } catch {
    /* no shell here; fall back to the web call */
  }
  buzz(pattern);
}

/** A gesture became active (long-press armed). The heaviest single tick. */
export function hapticArm(): void {
  pulse('arm', 15);
}

/** The selection changed to a different target, or a dial reached a
 *  detent. Deliberately light. */
export function hapticTick(): void {
  pulse('tick', 8);
}

/** A continuous dial crossed a magnitude boundary (seconds -> minutes -> ...). */
export function hapticStep(): void {
  pulse('step', 5);
}

/** An action was committed. */
export function hapticCommit(): void {
  pulse('commit', 20);
}

/** A destructive action was committed; distinct on purpose so a ban does not
 *  feel like a copy. */
export function hapticDestructive(): void {
  pulse('destructive', [15, 40, 15]);
}
