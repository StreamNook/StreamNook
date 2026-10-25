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

/** Where the playlist the origin serves now begins, or null while hls.js has no
 *  level details yet. */
function windowStart(hls: Hls): number | null {
  const start = hls.latestLevelDetails?.fragmentStart;
  return start != null && Number.isFinite(start) ? start : null;
}

/**
 * After a pause outlived the live window (pausePark 'resync'; loading already
 * restarted): the playhead makes one move to the live edge, and only once the
 * buffer holds the place it is moving to, which is the lower of hls.js's live
 * sync point and the buffered end less a small cushion. That place must also be
 * inside the window the origin serves now: video buffered before the pause is
 * not the live edge, so until new data covers the place the playhead stays put.
 * A seek into video that is not downloaded yet would freeze the element. After
 * the move it is left alone.
 *
 * The move waits for a playlist reload that started after the resync. The
 * stream counts as stuck, one window after the resync, when no new live data
 * arrived: a main fragment inside the reloaded playlist's window was never
 * buffered and the playhead has not reached that window, whatever the old
 * buffer is doing. With new live data it is also stuck when the video has not
 * fired `playing` and its time has not advanced. After the seek the window
 * starts again and the same playing check applies, so a freeze after the seek
 * is reported one window later. `onStuck` runs only while the video is not
 * paused. Returns the cancel.
 */
export function followLiveAfterResync(
  hls: Hls,
  video: HTMLVideoElement,
  onStuck: () => void,
  log?: (message: string) => void,
): () => void {
  let seeked = false;
  let playing = false;
  let reloaded = false; // a playlist arrived after the resync started
  let liveData = false; // a main fragment inside that playlist's window was buffered
  let baseline = video.currentTime;
  let timer: ReturnType<typeof setTimeout> | null = null;
  const arm = () => {
    if (timer !== null) clearTimeout(timer);
    timer = setTimeout(judge, RESYNC_WINDOW_MS);
  };
  const tryMove = () => {
    if (seeked || !reloaded) return;
    const pos = hls.liveSyncPosition;
    if (pos == null || !Number.isFinite(pos)) return;
    const first = windowStart(hls);
    if (first === null) return;
    const b = video.buffered;
    if (b.length === 0) return;
    const target = Math.min(pos, b.end(b.length - 1) - CUSHION_SECS);
    if (!Number.isFinite(target) || target <= video.currentTime + MIN_GAIN_SECS) return;
    if (target < first || !buffers(video, target)) return;
    seeked = true;
    video.currentTime = target;
    baseline = video.currentTime;
    playing = false;
    hls.off(Hls.Events.LEVEL_UPDATED, tryMove);
    hls.off(Hls.Events.FRAG_BUFFERED, tryMove);
    arm();
    log?.(`resync: playhead to the live edge (${target.toFixed(1)}s)`);
  };
  const onLevel = () => {
    reloaded = true;
  };
  const onFragment = (_e: unknown, data?: { frag?: { type?: string; start: number; duration: number } }) => {
    const f = data?.frag;
    if (!reloaded || !f || f.type !== 'main') return;
    const first = windowStart(hls);
    if (first !== null && f.start + f.duration > first) liveData = true;
  };
  const onPlaying = () => {
    playing = true;
  };
  const stop = () => {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
    hls.off(Hls.Events.LEVEL_UPDATED, onLevel);
    hls.off(Hls.Events.LEVEL_UPDATED, tryMove);
    hls.off(Hls.Events.FRAG_BUFFERED, tryMove);
    hls.off(Hls.Events.FRAG_BUFFERED, onFragment);
    video.removeEventListener('playing', onPlaying);
  };
  function judge() {
    timer = null;
    stop();
    if (video.paused) return;
    const first = windowStart(hls);
    const reached = reloaded && first !== null && video.currentTime >= first;
    if (!liveData && !reached) {
      onStuck();
      return;
    }
    if (playing || video.currentTime > baseline) return;
    onStuck();
  }
  hls.on(Hls.Events.LEVEL_UPDATED, onLevel); // before tryMove, so tryMove sees the reload
  hls.on(Hls.Events.LEVEL_UPDATED, tryMove);
  hls.on(Hls.Events.FRAG_BUFFERED, onFragment);
  hls.on(Hls.Events.FRAG_BUFFERED, tryMove);
  video.addEventListener('playing', onPlaying);
  arm();
  return stop;
}
