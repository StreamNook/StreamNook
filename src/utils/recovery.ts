/**
 * One recovery per player. The resume check, the freeze watchdog and the
 * behind-live watchdog all end in the function this returns. It restarts the
 * player the first time, and only when the player it was made for is still the
 * current one and nothing refuses a restart right now; a refused or stale call
 * does not use the recovery up, so a later trigger can still run it.
 * Returns whether this call started the restart.
 */
export function createRecovery<A extends unknown[]>(o: {
  /** Whether the player this guard belongs to is still the live one. */
  current: () => boolean;
  /** Whether a restart is not allowed right now (a swap or a switch is running). */
  refused: () => boolean;
  /** Stops the player's own checks before the restart. */
  stop: () => void;
  restart: (...args: A) => void;
}): (...args: A) => boolean {
  let recovering = false;
  return (...args) => {
    if (recovering || !o.current() || o.refused()) return false;
    recovering = true;
    o.stop();
    o.restart(...args);
    return true;
  };
}
