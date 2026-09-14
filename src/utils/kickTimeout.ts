// Kick's moderation API takes timeout durations in MINUTES, while the app's mod
// controls (keyboard ring, drag ramp, hover dock) all work in seconds. So a
// duration that isn't a whole number of minutes cannot be applied exactly.
//
// Two rules, both deliberate:
//   - Round UP, never down. Rounding to nearest would turn a 70-second timeout
//     into 60 seconds, i.e. quietly LESS than the moderator asked for.
//   - Floor of one minute, because zero would lift the timeout entirely.
//
// The UI must report what Kick will actually do rather than what was requested,
// which is what `kickAppliedSeconds` is for: a "30s" timeout really lasts a
// minute, and saying "30s" makes the app look broken when the user comes back.

/** Minutes to send to Kick for a timeout of `seconds`. */
export function kickTimeoutMinutes(seconds: number): number {
  return Math.max(1, Math.ceil(seconds / 60));
}

/** The duration Kick will really apply, for labels and toasts. */
export function kickAppliedSeconds(seconds: number): number {
  return kickTimeoutMinutes(seconds) * 60;
}
