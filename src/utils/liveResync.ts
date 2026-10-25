import Hls from 'hls.js';

/** How long a resync waits for the video to play again before the caller
 *  rebuilds the stream. */
export const RESYNC_WINDOW_MS = 8000;

/** A target less than this ahead of the playhead leaves it where it is. */
const MIN_GAIN_SECS = 1;

/** Seconds of buffer kept ahead of the playhead after the move, the same
 *  margin the Go Live button leaves. */
const CUSHION_SECS = 2;

/** How many seeks by anyone restart the freeze timer. */
const MAX_SEEK_REARMS = 3;

/** Seconds the playhead must have moved forward by playing for the stream not
 *  to count as frozen. */
const PROGRESS_SECS = 0.25;

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
 * The move waits for a playlist reload that started after the resync. Two
 * timers judge the stream. The data timer starts with the resync, is never
 * re-armed and, one window later, reports the stream stuck when no new live data
 * arrived: a main fragment inside the reloaded playlist's window was never
 * buffered and the playhead has not reached that window, whatever the old
 * buffer is doing. The freeze timer starts when new live data is first buffered
 * (or at this follower's own seek) and reports the stream stuck one window later
 * unless the playhead moved forward by playing. Any other seek of the element
 * restarts the freeze timer from where it landed, at most MAX_SEEK_REARMS
 * times; after that a seek never restarts it and the distance it jumped does not
 * count as progress. `onStuck` runs only while the video is not paused. Returns
 * the cancel.
 */
export function followLiveAfterResync(
  hls: Hls,
  video: HTMLVideoElement,
  onStuck: () => void,
  log?: (message: string) => void,
): () => void {
  let seeked = false;
  let reloaded = false; // a playlist arrived after the resync started
  let liveData = false; // a main fragment inside that playlist's window was buffered
  let baseline = video.currentTime; // where the freeze window measures progress from
  let jumped = 0; // seconds the playhead was moved by seeks that did not restart the window
  let lastPos = video.currentTime; // the playhead before the latest seek
  let rearms = 0;
  let dataTimer: ReturnType<typeof setTimeout> | null = null;
  let freezeTimer: ReturnType<typeof setTimeout> | null = null;
  const armFreeze = () => {
    if (freezeTimer !== null) clearTimeout(freezeTimer);
    freezeTimer = setTimeout(judgeFreeze, RESYNC_WINDOW_MS);
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
    jumped = 0;
    lastPos = video.currentTime;
    hls.off(Hls.Events.LEVEL_UPDATED, tryMove);
    hls.off(Hls.Events.FRAG_BUFFERED, tryMove);
    armFreeze();
    log?.(`resync: playhead to the live edge (${target.toFixed(1)}s)`);
  };
  const onLevel = () => {
    reloaded = true;
  };
  const onFragment = (_e: unknown, data?: { frag?: { type?: string; start: number; duration: number } }) => {
    const f = data?.frag;
    if (!reloaded || !f || f.type !== 'main') return;
    const first = windowStart(hls);
    if (first !== null && f.start + f.duration > first && !liveData) {
      liveData = true;
      armFreeze();
    }
  };
  const onTimeUpdate = () => {
    if (!video.seeking) lastPos = video.currentTime;
  };
  // Any seek, whoever made it (this follower, hls.js's own live sync, the
  // watchdog, a gap nudge), measures progress from where it landed. While the
  // freeze timer runs it restarts it, up to MAX_SEEK_REARMS times; past that
  // the seek only adds the distance it jumped, which is not progress.
  const onSeeking = () => {
    const now = video.currentTime;
    if (freezeTimer !== null && rearms >= MAX_SEEK_REARMS) {
      jumped += now - lastPos;
    } else {
      if (freezeTimer !== null) rearms += 1;
      baseline = now;
      jumped = 0;
      if (freezeTimer !== null) armFreeze();
    }
    lastPos = now;
  };
  const stop = () => {
    if (dataTimer !== null) {
      clearTimeout(dataTimer);
      dataTimer = null;
    }
    if (freezeTimer !== null) {
      clearTimeout(freezeTimer);
      freezeTimer = null;
    }
    hls.off(Hls.Events.LEVEL_UPDATED, onLevel);
    hls.off(Hls.Events.LEVEL_UPDATED, tryMove);
    hls.off(Hls.Events.FRAG_BUFFERED, tryMove);
    hls.off(Hls.Events.FRAG_BUFFERED, onFragment);
    video.removeEventListener('timeupdate', onTimeUpdate);
    video.removeEventListener('seeking', onSeeking);
  };
  // One window after the resync: no new live data and a playhead short of the
  // served window means the stream is stuck. Otherwise the freeze timer decides,
  // and starts here when nothing else started it.
  function judgeData() {
    dataTimer = null;
    const first = windowStart(hls);
    const reached = reloaded && first !== null && video.currentTime >= first;
    if (!liveData && !reached) {
      stop();
      if (!video.paused) onStuck();
      return;
    }
    if (freezeTimer === null) armFreeze();
  }
  function judgeFreeze() {
    freezeTimer = null;
    stop();
    if (video.paused) return;
    if (video.currentTime - baseline - jumped > PROGRESS_SECS) return;
    onStuck();
  }
  hls.on(Hls.Events.LEVEL_UPDATED, onLevel); // before tryMove, so tryMove sees the reload
  hls.on(Hls.Events.LEVEL_UPDATED, tryMove);
  hls.on(Hls.Events.FRAG_BUFFERED, onFragment);
  hls.on(Hls.Events.FRAG_BUFFERED, tryMove);
  video.addEventListener('timeupdate', onTimeUpdate);
  video.addEventListener('seeking', onSeeking);
  dataTimer = setTimeout(judgeData, RESYNC_WINDOW_MS);
  return stop;
}
