import Hls from 'hls.js';

/** How long a resync waits for the video to play again before the caller
 *  rebuilds the stream. */
export const RESYNC_WINDOW_MS = 8000;

/** A target less than this ahead of the playhead leaves it where it is. */
const MIN_GAIN_SECS = 1;

/** Seconds of buffer kept ahead of the playhead after the move, the same
 *  margin the Go Live button leaves. */
const CUSHION_SECS = 2;

/** Whether some buffered range holds `t`. */
function buffers(video: HTMLVideoElement, t: number): boolean {
  const b = video.buffered;
  for (let i = 0; i < b.length; i++) {
    if (b.start(i) <= t && t <= b.end(i)) return true;
  }
  return false;
}

export interface ResyncOptions {
  /** False when the caller moves the playhead itself (a MultiNook tile held at
   *  a shared delay); the stuck check still runs. */
  seek?: boolean;
}

/**
 * After a pause outlived the live window (pausePark 'resync'; loading already
 * restarted): the playhead makes one move to the live edge, and only once the
 * buffer holds the place it is moving to, which is the lower of hls.js's live
 * sync point and the buffered end less a small cushion. A seek into video that
 * is not downloaded yet would freeze the element, so until a buffered range
 * covers that place the playhead stays put. After the move it is left alone.
 *
 * The stream counts as stuck when, by the end of the window, the video has not
 * fired `playing` and its time has not advanced (an upstream that refused the
 * parked URL, for one): `onStuck` runs, and only while the video is not paused.
 * Returns the cancel.
 */
export function followLiveAfterResync(
  hls: Hls,
  video: HTMLVideoElement,
  onStuck: () => void,
  log?: (message: string) => void,
  options: ResyncOptions = {},
): () => void {
  const wantSeek = options.seek !== false;
  let seeked = !wantSeek;
  let playing = false;
  let baseline = video.currentTime;
  let timer: ReturnType<typeof setTimeout> | null = null;
  const tryMove = () => {
    if (seeked) return;
    const pos = hls.liveSyncPosition;
    if (pos == null || !Number.isFinite(pos)) return;
    const b = video.buffered;
    if (b.length === 0) return;
    const target = Math.min(pos, b.end(b.length - 1) - CUSHION_SECS);
    if (!Number.isFinite(target) || target <= video.currentTime + MIN_GAIN_SECS) return;
    if (!buffers(video, target)) return;
    seeked = true;
    video.currentTime = target;
    baseline = video.currentTime;
    hls.off(Hls.Events.LEVEL_UPDATED, tryMove);
    hls.off(Hls.Events.FRAG_BUFFERED, tryMove);
    log?.(`resync: playhead to the live edge (${target.toFixed(1)}s)`);
  };
  const onPlaying = () => {
    playing = true;
  };
  const stop = () => {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
    hls.off(Hls.Events.LEVEL_UPDATED, tryMove);
    hls.off(Hls.Events.FRAG_BUFFERED, tryMove);
    video.removeEventListener('playing', onPlaying);
  };
  if (wantSeek) {
    hls.on(Hls.Events.LEVEL_UPDATED, tryMove);
    hls.on(Hls.Events.FRAG_BUFFERED, tryMove);
  }
  video.addEventListener('playing', onPlaying);
  timer = setTimeout(() => {
    timer = null;
    stop();
    if (playing || video.currentTime > baseline || video.paused) return;
    onStuck();
  }, RESYNC_WINDOW_MS);
  return stop;
}
