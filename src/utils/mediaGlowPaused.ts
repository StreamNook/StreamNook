/** A paused video whose frame at `currentTime` was already read back shows
 *  nothing new: skip the readback (a GPU round trip) until it plays or seeks. */
export function pausedFrameAlreadyRead(
  el: Pick<HTMLVideoElement, 'paused' | 'seeking' | 'currentTime'>,
  readAt: number,
): boolean {
  return el.paused && !el.seeking && el.currentTime === readAt;
}
