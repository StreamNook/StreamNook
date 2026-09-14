import { useEffect, useState, type RefObject } from 'react';
import type { MutedRange } from '../types';

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
      const t = video.currentTime;
      // Ranges are sorted and non-overlapping (Rust guarantees it) and there
      // are a couple of dozen at most, so a scan is cheaper than a search.
      const found = ranges.find((r) => t >= r.start_secs && t < r.end_secs) ?? null;
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
