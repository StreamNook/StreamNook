import Hls from 'hls.js';
import type { ErrorData } from 'hls.js';

/** The slice of an hls.js instance this recovery touches. */
type HlsLike = Pick<Hls, 'config' | 'once' | 'startLoad'>;

/**
 * Recover from a live playlist refresh that hls.js rejected for naming a
 * different segment URI (or discontinuity) under a sequence number it already
 * holds ("media sequence mismatch" / "discontinuity sequence mismatch").
 *
 * hls.js keeps the playlist it had before the rejected refresh, and with a
 * single rendition the rejection is fatal, so loading stops. A plain
 * `startLoad()` then compares every later refresh against that same stale
 * copy and rejects it again, until the live window has slid entirely past it:
 * about half a minute of stall. Accepting exactly one refresh replaces the
 * stale copy, and playback continues from the buffer it already has.
 *
 * Returns true when it handled the error. Only the sequence mismatch is taken:
 * a playlist that fails to parse at all is a different failure and keeps the
 * caller's normal handling.
 */
export function recoverFromSequenceMismatch(hls: HlsLike, data: ErrorData): boolean {
  if (data.details !== Hls.ErrorDetails.LEVEL_PARSING_ERROR) return false;
  const message = data.error?.message ?? '';
  if (!message.includes('sequence mismatch')) return false;
  hls.config.ignorePlaylistParsingErrors = true;
  hls.once(Hls.Events.LEVEL_UPDATED, () => {
    hls.config.ignorePlaylistParsingErrors = false;
  });
  hls.startLoad();
  return true;
}
