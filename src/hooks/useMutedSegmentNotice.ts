import { useEffect, useState, type RefObject } from 'react';
import type { MutedRange } from '../types';

/** The muted range the time `t` (seconds) falls inside, or null. Ranges are
 *  sorted and non-overlapping (Rust guarantees it). */
export function mutedRangeAt(ranges: MutedRange[] | undefined, t: number): MutedRange | null {
  if (!ranges || !Number.isFinite(t)) return null;
  return ranges.find((r) => t >= r.start_secs && t < r.end_secs) ?? null;
}

/** Where "Skip" lands for a muted range (SN-282): the moment the sound comes
 *  back. A range that runs to the end of the video stops just short of the
 *  end, so the player shows its last frame instead of ending on the click. */
export function skipMutedTarget(range: MutedRange, duration: number): number {
  if (Number.isFinite(duration) && duration > 0 && range.end_secs >= duration - 0.5) {
    return Math.max(range.start_secs, duration - 0.5);
  }
  return range.end_secs;
}

/**
 * Which muted range the playhead is currently inside, or null.
 *
 * Rides the `timeupdate` the media element already fires rather than adding a
 * timer, and commits state only when the CONTAINING RANGE changes, so a
 * three-minute mute costs two renders instead of 4 Hz of them. Attaches no
 * listener at all when there are no ranges.
 */
export function useMutedSegmentNotice(
  videoRef: RefObject<HTMLVideoElement | null>,
  ranges: MutedRange[],
): MutedRange | null {
  const [active, setActive] = useState<MutedRange | null>(null);

  useEffect(() => {
    const video = videoRef.current;
    if (!video || ranges.length === 0) return;

    const check = () => {
      // A couple of dozen ranges at most, so a scan is cheaper than a search.
      const found = mutedRangeAt(ranges, video.currentTime);
      setActive((prev) => (prev?.start_secs === found?.start_secs ? prev : found));
    };

    check();
    video.addEventListener('timeupdate', check);
    video.addEventListener('seeked', check);
    return () => {
      video.removeEventListener('timeupdate', check);
      video.removeEventListener('seeked', check);
    };
  }, [videoRef, ranges]);

  // Derived rather than stored: with no ranges nothing can be active, so there
  // is no need to clear stale state with a setState in the effect body. When
  // the list changes to a different non-empty one, the effect's own opening
  // `check()` recomputes against it.
  return ranges.length === 0 ? null : active;
}
