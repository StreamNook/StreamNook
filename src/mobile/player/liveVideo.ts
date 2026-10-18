// A module-scope handle on the live <video> element.
//
// The clip sheet needs to DUCK the live stream while a clip plays: one phone,
// one speaker, so two simultaneous audio streams is just noise. The element
// itself is a local ref inside MobilePlayer and nothing exposed it.
//
// Module scope is the honest shape here rather than a leak: useMobileHlsEngine
// already documents (engine header) that there is exactly ONE player - one hook
// call site, in one MobilePlayer, in one WatchScreen. This registry inherits
// that same single-instance assumption, so if a second live player surface is
// ever added, this moves into the hook alongside the engine state.
//
// Deliberately NOT a `window.__sn*` global and NOT a `data-` attribute query:
// the clip sheet mounts its own <video>, so a DOM query for "the video" would
// be ambiguous exactly when it matters.

import { useSyncExternalStore } from 'react';

let liveVideo: HTMLVideoElement | null = null;

// Whether the viewer has the live element paused, for the watch layer's
// keep-awake: a paused stream lets the screen sleep like any other paused
// video. Driven by `play`/`pause` (intent), not `paused` (which is also true
// while a start is still resolving, when the screen should stay on).
let paused = false;
const pausedListeners = new Set<() => void>();
function setPaused(next: boolean): void {
  if (paused === next) return;
  paused = next;
  pausedListeners.forEach((l) => l());
}
const onPlay = () => setPaused(false);
const onPause = () => setPaused(true);

/** Called by MobilePlayer's ref callback. Null on unmount. */
export function setLiveVideo(el: HTMLVideoElement | null): void {
  if (liveVideo === el) return;
  if (liveVideo) {
    liveVideo.removeEventListener('play', onPlay);
    liveVideo.removeEventListener('pause', onPause);
  }
  liveVideo = el;
  // A fresh element is about to play; nothing is paused until it says so.
  setPaused(false);
  if (el) {
    el.addEventListener('play', onPlay);
    el.addEventListener('pause', onPause);
  }
}

/** True while the live element is paused by the viewer (or the system). */
export function useLiveVideoPaused(): boolean {
  return useSyncExternalStore(
    (listener) => {
      pausedListeners.add(listener);
      return () => pausedListeners.delete(listener);
    },
    () => paused,
  );
}

export function getLiveVideo(): HTMLVideoElement | null {
  return liveVideo;
}

/**
 * Mute the live stream and hand back a restore function.
 *
 * Mute, NOT pause. Pausing a live stream drops it off the live edge and forces
 * a catch-up (or a stall) when the clip closes; muting keeps it running exactly
 * where it was. Restoring the PREVIOUS muted state rather than unconditionally
 * unmuting matters because the viewer may already have been watching muted.
 *
 * Safe to call with no live element (clip opened from a non-watch surface): the
 * returned restore is then a no-op.
 */
export function duckLiveAudio(): () => void {
  const el = liveVideo;
  if (!el) return () => {};
  const wasMuted = el.muted;
  el.muted = true;
  return () => {
    // The element may have been torn down while the clip played.
    if (liveVideo === el) el.muted = wasMuted;
  };
}
