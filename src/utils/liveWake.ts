/** What a video event does to the LIVE readout. The readout recomputes on a
 *  250 ms tick that runs only while the video plays (or seeks) and the window
 *  is shown; play, pause and seeks also recompute at once, so the LIVE button
 *  never waits out a tick (a pause would otherwise leave it reading "live"). */
export interface LiveWake {
  /** Recompute the readout and the LIVE button now. */
  recompute: boolean;
}

/** Events that change what the readout says. */
const RECOMPUTE_EVENTS = new Set(['play', 'pause', 'seeking', 'seeked']);

/** The wake rule for `event`. `timeupdate` recomputes nothing; it only lets
 *  the caller re-check whether the tick should run. */
export function liveWake(event: string): LiveWake {
  return { recompute: RECOMPUTE_EVENTS.has(event) };
}

/** Whether the readout's 250 ms tick should run: a live stream that is
 *  playing or seeking, in a window that is shown. */
export function liveTickWanted(s: { live: boolean; paused: boolean; seeking: boolean; hidden: boolean }): boolean {
  return s.live && !s.hidden && !(s.paused && !s.seeking);
}
