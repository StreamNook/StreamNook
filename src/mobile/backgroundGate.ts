// Whether the app is currently in the background.
//
// Its own module rather than a flag inside lifecycle.ts, because the polls that
// read it (drop progress, channel points, pinned messages) would otherwise all
// import the lifecycle module and drag its dynamic imports along with them.
//
// Polls SKIP A TICK rather than tearing their interval down. Clearing and
// re-arming intervals across every background and foreground cycle means the
// resume path has to rebuild them all in the same frame the app is already busy
// restoring, and a missed tick costs nothing: every one of these polls is a
// refresh of state that is re-read on the next tick anyway.
//
// Playback is deliberately not gated. Backgrounding a stream to keep listening
// is a normal way to use a Twitch client. See lifecycle.ts.

let backgrounded = false;

/** True while the app is hidden and NOT in picture-in-picture. */
export function isBackgrounded(): boolean {
  return backgrounded;
}

/** Driven by lifecycle.ts, which owns the visibilitychange handling. */
export function setBackgrounded(value: boolean): void {
  backgrounded = value;
}
