import { useEffect, useState, type RefObject } from 'react';
import { createPortal } from 'react-dom';
import type { MutedRange } from '../types';

/**
 * Muted-audio bands drawn over a VOD's Plyr progress bar.
 *
 * Portaled into Plyr's `.plyr__progress` slot, which is how Plyr itself adds
 * bar annotations (`.plyr__progress__marker`). Deliberately WITHOUT the
 * `.sn-timeline-host` class BroadcastTimeline uses: that class hides Plyr's
 * own range input, and a VOD needs it for seeking.
 *
 * Pure presentation. Rust merges, sorts and clamps the ranges, so there is
 * nothing to compute here beyond the pixel mapping.
 */
export default function VodMutedMarks({
  host,
  ranges,
  videoRef,
  fallbackLengthSecs,
}: {
  /** Plyr's `.plyr__progress` element. */
  host: HTMLElement | null;
  ranges: MutedRange[];
  videoRef: RefObject<HTMLVideoElement | null>;
  /** `length_seconds` from Rust, used only until the element reports one. */
  fallbackLengthSecs: number;
}) {
  // The denominator has to be the element's own duration, because that is what
  // drives the bar these marks sit on. GQL's integer `lengthSeconds` is close
  // (43691 vs the playlist's 43691.150) but it is a different number in a
  // different space, and it can be missing entirely.
  const [duration, setDuration] = useState(0);

  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    const read = () =>
      setDuration(Number.isFinite(video.duration) && video.duration > 0 ? video.duration : 0);
    read();
    video.addEventListener('loadedmetadata', read);
    video.addEventListener('durationchange', read);
    return () => {
      video.removeEventListener('loadedmetadata', read);
      video.removeEventListener('durationchange', read);
    };
  }, [videoRef]);

  const lengthSecs = duration || fallbackLengthSecs;
  if (!host || lengthSecs <= 0 || ranges.length === 0) return null;

  return createPortal(
    <div className="sn-muted-marks" aria-hidden="true">
      {ranges.map((range) => {
        const left = (range.start_secs / lengthSecs) * 100;
        if (!(left < 100)) return null;
        // A 180 s mute on a 12 h VOD is 0.4% of the bar. Floor the width so a
        // short mute stays a visible mark instead of a sub-pixel sliver.
        const width = Math.max(0.4, ((range.end_secs - range.start_secs) / lengthSecs) * 100);
        return (
          <div
            key={range.start_secs}
            className="sn-muted-mark"
            style={{ left: `${left}%`, width: `${Math.min(width, 100 - left)}%` }}
          />
        );
      })}
    </div>,
    host,
  );
}
