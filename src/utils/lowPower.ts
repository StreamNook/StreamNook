// How cheaply this window draws, from two sources that never write each other:
//  - battery: the phone's system Battery Saver (MainActivity -> sn:low-power,
//    installed by src/mobile/lowPower.ts). Desktop never sets it.
//  - performance: desktop Performance mode (Settings > Interface), applied by
//    src/utils/performanceMode.ts from what Rust decides.
// Each source says three things:
//  - quiet: decorative loops rest (globals.css `html[data-sn-low-power]`). On
//    the phone, Battery Saver also steps Motion down to Reduced (MotionScope).
//    Performance mode's quiet also finishes CSS animations at once
//    (`html[data-sn-perf]`), so entrances appear instantly on desktop.
//  - still: animated emotes, 7TV paints, badges and pictures draw their first
//    frame and chat GIFs show as chips (`html[data-sn-still-chat]`).
//  - flat: no backdrop blur and no player glow (desktop Performance mode only).
import { useSyncExternalStore } from 'react';

export type LowPowerSource = 'battery' | 'performance';
export interface LowPowerLook {
  quiet: boolean;
  still: boolean;
  flat: boolean;
}

const OFF: LowPowerLook = { quiet: false, still: false, flat: false };
const sources = new Map<LowPowerSource, LowPowerLook>();
const listeners = new Set<() => void>();

let quiet = false;
let batteryQuiet = false;
let performanceQuiet = false;
let still = false;
let flat = false;

function derive(): void {
  const all = [...sources.values()];
  const nextQuiet = all.some((l) => l.quiet);
  const nextBattery = sources.get('battery')?.quiet ?? false;
  const nextPerformance = sources.get('performance')?.quiet ?? false;
  const nextStill = all.some((l) => l.still);
  const nextFlat = all.some((l) => l.flat);
  if (
    nextQuiet === quiet &&
    nextBattery === batteryQuiet &&
    nextPerformance === performanceQuiet &&
    nextStill === still &&
    nextFlat === flat
  ) {
    return;
  }
  quiet = nextQuiet;
  batteryQuiet = nextBattery;
  performanceQuiet = nextPerformance;
  still = nextStill;
  flat = nextFlat;
  if (typeof document !== 'undefined') {
    const data = document.documentElement.dataset;
    if (quiet) data.snLowPower = 'true';
    else delete data.snLowPower;
    if (performanceQuiet) data.snPerf = 'true';
    else delete data.snPerf;
    if (still) data.snStillChat = 'true';
    else delete data.snStillChat;
  }
  listeners.forEach((l) => l());
}

/** One source's look. Returns whether that source changed. */
export function setLowPowerSource(source: LowPowerSource, look: LowPowerLook): boolean {
  const was = sources.get(source) ?? OFF;
  if (was.quiet === look.quiet && was.still === look.still && was.flat === look.flat) return false;
  sources.set(source, { quiet: look.quiet, still: look.still, flat: look.flat });
  derive();
  return true;
}

/** The phone's Battery Saver: quiet and still together, never flat.
 *  Returns whether anything changed. */
export function setLowPower(next: boolean): boolean {
  return setLowPowerSource('battery', { quiet: next, still: next, flat: false });
}

export function isLowPower(): boolean {
  return quiet;
}
export function isBatteryLowPower(): boolean {
  return batteryQuiet;
}
export function isPerformanceQuiet(): boolean {
  return performanceQuiet;
}
export function isStillChat(): boolean {
  return still;
}
export function isFlat(): boolean {
  return flat;
}

export function subscribeLowPower(l: () => void): () => void {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
}

// The getter doubles as the server snapshot, so a static render (the tests'
// renderToStaticMarkup) sees the current state, as before.
export function useLowPower(): boolean {
  return useSyncExternalStore(subscribeLowPower, isLowPower, isLowPower);
}
export function useBatteryLowPower(): boolean {
  return useSyncExternalStore(subscribeLowPower, isBatteryLowPower, isBatteryLowPower);
}
export function useStillChat(): boolean {
  return useSyncExternalStore(subscribeLowPower, isStillChat, isStillChat);
}
export function useFlat(): boolean {
  return useSyncExternalStore(subscribeLowPower, isFlat, isFlat);
}
