// Segments on a track, the chosen one a darker pill set into it that glides
// between them (the look of the desktop Home tab strip and the phone's tab
// bar). The track is the theme's own raised surface and opaque, so content
// scrolling under it never shows through and the unselected labels keep the
// contrast the theme measured them on; `.sn-segment-track` in mobile.css
// carries its edge.
import { useId } from 'react';
import { motion } from 'framer-motion';

export interface Segment<T extends string> {
  value: T;
  label: string;
  /** A small count after the label (unread, say); none at 0. */
  count?: number;
}

/** A segment's count, a step quieter than its label and never a corner dot. */
function segmentCount(count: number | undefined) {
  if (!count) return null;
  return <span className="ml-1 text-[0.85em] font-medium tabular-nums opacity-75">{count > 99 ? '99+' : count}</span>;
}

export function SegmentedControl<T extends string>({
  segments,
  value,
  onChange,
  label,
  fill = false,
}: {
  segments: Segment<T>[];
  value: T;
  onChange: (next: T) => void;
  /** What the group chooses, for screen readers. */
  label: string;
  /** Span the row: each segment gets its label's width plus an equal share of
      what is left, so a long label never wraps beside short ones. On a narrow
      screen the labels step down from 14 px rather than break. */
  fill?: boolean;
}) {
  // One pill per control: two controls on screen must never trade pills.
  const pillId = useId();
  return (
    <div role="radiogroup" aria-label={label} className="sn-segment-track flex items-center gap-1 p-1">
      {segments.map((s) => {
        const on = s.value === value;
        return (
          <button
            key={s.value}
            role="radio"
            aria-checked={on}
            onClick={() => {
              if (!on) onChange(s.value);
            }}
            className={`relative rounded-full py-1.5 ${fill ? 'flex-auto px-1.5' : 'px-3.5'}`}
          >
            {on && (
              <motion.span
                layoutId={`segment-pill-${pillId}`}
                className="absolute inset-0 glaze-selected"
                transition={{ type: 'spring', stiffness: 350, damping: 30 }}
              />
            )}
            {/* Unselected labels sit on the raised surface, where muted text
                falls under 4.5:1, so they use secondary text. The bold label is
                drawn invisibly underneath so the segment keeps one width
                whichever weight it shows, and the track never shifts. A label
                is one line always. */}
            <span
              className={`relative z-10 grid whitespace-nowrap ${fill ? 'text-[min(14px,3.6vw)]' : 'text-sm'}`}
            >
              <span aria-hidden className="invisible col-start-1 row-start-1 font-semibold">
                {s.label}
                {segmentCount(s.count)}
              </span>
              <span
                className={`col-start-1 row-start-1 text-center transition-colors ${
                  on ? 'font-semibold text-textPrimary' : 'text-textSecondary'
                }`}
              >
                {s.label}
                {segmentCount(s.count)}
              </span>
            </span>
          </button>
        );
      })}
    </div>
  );
}
