// System Battery Saver, as the phone reports it (MainActivity -> sn:low-power,
// installed by src/mobile/lowPower.ts). Desktop never sets it: false there.
// While on: animated emotes draw their first frame, decorative loops stop
// (mobile.css `html[data-sn-low-power]`), Motion acts as Reduced, and the
// window asks for 60 Hz (native side).
import { useSyncExternalStore } from 'react';

let lowPower = false;
const listeners = new Set<() => void>();

export function isLowPower(): boolean {
  return lowPower;
}

/** Returns whether anything changed. */
export function setLowPower(next: boolean): boolean {
  if (lowPower === next) return false;
  lowPower = next;
  if (typeof document !== 'undefined') {
    if (next) document.documentElement.dataset.snLowPower = 'true';
    else delete document.documentElement.dataset.snLowPower;
  }
  listeners.forEach((l) => l());
  return true;
}

export function subscribeLowPower(l: () => void): () => void {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
}

export function useLowPower(): boolean {
  return useSyncExternalStore(subscribeLowPower, isLowPower, isLowPower);
}
