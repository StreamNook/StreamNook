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

let liveVideo: HTMLVideoElement | null = null;

/** Called by MobilePlayer's ref callback. Null on unmount. */
export function setLiveVideo(el: HTMLVideoElement | null): void {
  liveVideo = el;
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
