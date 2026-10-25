import Hls from 'hls.js';

/** How long a resync keeps moving the playhead to the reloaded edge, and how
 *  long it waits for a fragment before the caller rebuilds the stream. */
export const RESYNC_WINDOW_MS = 8000;

/** A reload whose sync point is less than this ahead leaves the playhead. */
const MIN_GAIN_SECS = 1;

/**
 * After a pause outlived the live window (pausePark 'resync'; loading already
 * restarted): every playlist reload inside the window moves the playhead to
 * hls.js's live sync point when that is ahead of it, which is where a cold
 * start begins. A parked relay first serves its held edge and then the rebuilt
 * one, so the playhead follows both. With no fragment loaded by the end of the
 * window the stream is not coming back on this instance (an upstream that
 * refused the parked URL, for one): `onStuck`. Returns the cancel.
 */
export function followLiveAfterResync(
  hls: Hls,
  video: HTMLVideoElement,
  onStuck: () => void,
  log?: (message: string) => void,
): () => void {
  let fragLoaded = false;
  let timer: ReturnType<typeof setTimeout> | null = null;
  const onLevel = () => {
    const pos = hls.liveSyncPosition;
    if (pos != null && Number.isFinite(pos) && pos > video.currentTime + MIN_GAIN_SECS) {
      video.currentTime = pos;
      log?.(`resync: playhead to the live edge (${pos.toFixed(1)}s)`);
    }
  };
  const onFrag = () => {
    fragLoaded = true;
  };
  const stop = () => {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
    hls.off(Hls.Events.LEVEL_UPDATED, onLevel);
    hls.off(Hls.Events.FRAG_LOADED, onFrag);
  };
  hls.on(Hls.Events.LEVEL_UPDATED, onLevel);
  hls.on(Hls.Events.FRAG_LOADED, onFrag);
  timer = setTimeout(() => {
    timer = null;
    stop();
    if (!fragLoaded) onStuck();
  }, RESYNC_WINDOW_MS);
  return stop;
}
