// The hold rule for the pins block (SN-284, D16). While a pointer moved on the
// block in the last 2 seconds, or a finger is down on it, nothing under the
// pointer may be removed or move: a pin whose time is up stays drawn, and a
// pin that would be inserted above what the pointer is on waits. New pins that
// arrive at the bottom (the newest end) still appear. When the hold lets go the
// block shows Rust's answer as it is.

import type { MentionPin } from '../../types';

/** How long after the last pointer move the hold lasts. */
export const HOLD_MS = 2000;

/**
 * What a held block draws: the pins it held, each with its newest data from
 * `live` (or as held when Rust dropped it), then the pins that are new and come
 * after the last held pin in Rust's order. A new pin that sorts above a held
 * one is left out until the hold lets go.
 */
export function mergeHeld(frozen: readonly MentionPin[], live: readonly MentionPin[]): MentionPin[] {
  const liveIndex = new Map(live.map((p, i) => [p.entry.id, i] as const));
  const frozenIds = new Set(frozen.map((p) => p.entry.id));
  let lastHeldAt = -1;
  for (const p of frozen) {
    const at = liveIndex.get(p.entry.id);
    if (at !== undefined && at > lastHeldAt) lastHeldAt = at;
  }
  const kept = frozen.map((p) => live[liveIndex.get(p.entry.id) ?? -1] ?? p);
  const appended = live.filter((p, i) => !frozenIds.has(p.entry.id) && i > lastHeldAt);
  return [...kept, ...appended];
}

/** The "Mon 3:04 PM"-style day prefix for a pin said on another day, else null. */
export function dayLabelFor(atMs: number, now: Date = new Date(), locale?: string): string | null {
  const at = new Date(atMs);
  if (at.toDateString() === now.toDateString()) return null;
  const sameWeek = now.getTime() - at.getTime() < 6 * 24 * 3600 * 1000;
  return at.toLocaleDateString(locale, sameWeek ? { weekday: 'short' } : { month: 'short', day: 'numeric' });
}
