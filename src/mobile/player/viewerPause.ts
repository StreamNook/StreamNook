// Which pauses the viewer chose. Every control that pauses on the viewer's
// word (the player's button, the media card, a headset) notes it just before
// it pauses; the player reads the note on the `pause` event that follows. A
// pause with no note came from somewhere else: Android's WebView pauses on its
// own when a call or another app takes the sound, and the app pauses when you
// leave it with Background play off. Those come back at the live edge sooner
// (pausePolicy's INTERRUPTION_RESYNC_MS).

/** A note older than this belongs to some earlier pause. The `pause` event is
 *  queued a task after `pause()`, so a second is plenty. */
const NOTE_FRESH_MS = 1_000;

let notedAt: number | null = null;

/** The viewer is pausing the stream. Call just before `video.pause()`. */
export function noteViewerPause(now = Date.now()): void {
  notedAt = now;
}

/** Whether the pause happening now is the viewer's. Reading spends the note. */
export function takeViewerPause(now = Date.now()): boolean {
  const chosen = notedAt !== null && now - notedAt >= 0 && now - notedAt <= NOTE_FRESH_MS;
  notedAt = null;
  return chosen;
}
