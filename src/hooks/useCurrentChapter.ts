import { useEffect, useState, type RefObject } from 'react';
import type { VodChapter } from '../types';

/**
 * Which chapter the playhead is currently inside, or null.
 *
 * Rides the `timeupdate` the media element already fires rather than adding a
 * timer, and commits state only when the CONTAINING CHAPTER changes, so an
 * hour-long chapter costs two renders instead of 4 Hz of them. Attaches no
 * listener at all with fewer than two chapters (one chapter is the category
 * the card already shows).
 */
export function useCurrentChapter(
  videoRef: RefObject<HTMLVideoElement | null>,
  chapters: VodChapter[],
): VodChapter | null {
  const [active, setActive] = useState<VodChapter | null>(null);

  useEffect(() => {
    const video = videoRef.current;
    if (!video || chapters.length < 2) return;

    const check = () => {
      const t = video.currentTime;
      // Sorted and non-overlapping (Rust guarantees it), a handful at most.
      // Past the last chapter's integer end (the playlist runs a little
      // longer than GQL's length) the last chapter still applies.
      const found = chapters.find((c) => t >= c.start_secs && t < c.end_secs) ?? chapters[chapters.length - 1];
      setActive((prev) => (prev?.start_secs === found.start_secs ? prev : found));
    };

    check();
    video.addEventListener('timeupdate', check);
    video.addEventListener('seeked', check);
    return () => {
      video.removeEventListener('timeupdate', check);
      video.removeEventListener('seeked', check);
    };
  }, [videoRef, chapters]);

  // Derived rather than stored: with too few chapters nothing can be active,
  // so there is no stale state to clear in the effect body.
  return chapters.length < 2 ? null : active;
}
