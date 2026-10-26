/** What a video event does to the LIVE readout loop. The loop recomputes the
 *  readout and the LIVE button at most every 250 ms, and sleeps while the video
 *  is paused. */
export interface LiveWake {
  /** Let the next frame recompute at once instead of waiting out the 250 ms gate. */
  resetGate: boolean;
  /** Schedule a frame now; false when one is already pending. */
  schedule: boolean;
}

/** Events that change what the readout says, so they always get a fresh
 *  recompute even while a frame is already pending. */
const RECOMPUTE_EVENTS = new Set(['play', 'pause', 'seeking', 'seeked']);

/** The wake rule for `event`. `play`, `pause`, `seeking` and `seeked` reset the
 *  gate whether or not a frame is pending, so the frame that runs after them
 *  recomputes (a pause with a frame in flight would otherwise land inside the
 *  gate and leave the button reading "live"). `timeupdate` only wakes a loop
 *  that is asleep. */
export function liveWake(event: string, frameScheduled: boolean): LiveWake {
  const recompute = RECOMPUTE_EVENTS.has(event);
  return {
    resetGate: recompute || !frameScheduled,
    schedule: !frameScheduled,
  };
}
