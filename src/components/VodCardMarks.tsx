import type { VodProgressSummary } from '../types';
import { vodProgressFraction } from '../utils/vodProgress';

/**
 * Watched-progress marks for a VOD card. Rust joins `progress` onto every
 * video row it returns, so these are pure presentation: no fetch, no store.
 */

/** Thin bar along the bottom edge of a thumbnail. Renders nothing when the
 *  video was never watched. */
export function VodProgressBar({
  progress,
  lengthSeconds,
}: {
  progress: VodProgressSummary | undefined;
  lengthSeconds: number | undefined;
}) {
  const frac = vodProgressFraction(progress, lengthSeconds);
  if (frac == null) return null;
  return (
    <div
      className="pointer-events-none absolute inset-x-0 bottom-0 h-[3px] bg-white/20"
      aria-label={progress?.completed ? 'Watched' : `Watched ${Math.round(frac * 100)}%`}
    >
      <div className="h-full bg-accent" style={{ width: `${frac * 100}%` }} />
    </div>
  );
}

/** Red pill for a VOD whose broadcast is still live (the recording grows). */
export function VodRecordingBadge({ status }: { status: string | undefined }) {
  if (status !== 'recording') return null;
  return (
    <div className="glass-badge absolute top-1.5 right-1.5 flex items-center gap-1 rounded px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-white">
      <span className="h-1.5 w-1.5 rounded-full bg-red-500" />
      Live now
    </div>
  );
}
