import type { ChatDesignSettings } from '../types';

/** The delay row's range and default, shared by the settings row and the reader. */
export const CONFETTI_CLEAR_MIN_S = 3;
export const CONFETTI_CLEAR_MAX_S = 15;
export const CONFETTI_CLEAR_DEFAULT_S = 5;

/**
 * How long a landed piece of hype train confetti rests before it fades, in ms,
 * or null when it stays until it is brushed away (Confetti cleanup is off, as
 * it is on a fresh install).
 */
export function confettiClearMs(cd: ChatDesignSettings | undefined): number | null {
  if (!(cd?.confetti_auto_clear ?? false)) return null;
  const s = cd?.confetti_clear_seconds ?? CONFETTI_CLEAR_DEFAULT_S;
  const clamped = Math.min(CONFETTI_CLEAR_MAX_S, Math.max(CONFETTI_CLEAR_MIN_S, Number.isFinite(s) ? s : CONFETTI_CLEAR_DEFAULT_S));
  return clamped * 1000;
}
