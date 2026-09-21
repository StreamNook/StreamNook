export type SwapVerdict = 'apply' | 'discard' | 'discard-and-stop';

/**
 * What to do with an audio-only swap that has just resolved.
 *
 * `change_stream_quality` is `start_stream` underneath: by the time it
 * returns, the Rust relay is up for the channel the swap was issued for. Three
 * things can have happened in the meantime:
 *
 *  - a newer start (a channel switch, a rewind, a return to live) bumped the
 *    sequence: discard. Its own resolve is ordered after ours and owns the
 *    relay, so there is nothing to undo.
 *  - same sequence but the stream is closed: `stopStream` clears the URL
 *    first and the stream object last, so either reads as closed. Discard AND
 *    stop the relay we just brought up, or the phone keeps decoding audio for
 *    a stream nobody is watching, with no player, no overlay and no card.
 *  - otherwise apply.
 */
export function transientSwapVerdict(a: {
  seqAtStart: number;
  seqNow: number;
  streamUrlNow: string | null;
  loginAtStart: string;
  loginNow: string | null | undefined;
}): SwapVerdict {
  if (a.seqAtStart !== a.seqNow) return 'discard';
  if (!a.streamUrlNow || a.loginNow !== a.loginAtStart) return 'discard-and-stop';
  return 'apply';
}
