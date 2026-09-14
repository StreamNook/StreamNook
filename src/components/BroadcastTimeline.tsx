import { useCallback, useEffect, useRef, useState, type RefObject } from 'react';
import { createPortal } from 'react-dom';
import { formatAgo, formatVodTime } from '../utils/vodProgress';
import { broadcastScale, formatBehindShort } from '../utils/broadcastScale';
import { createLiveEdgeTracker } from '../utils/liveEdge';

/**
 * The broadcast timeline for a live Twitch stream: a scrubber that spans the
 * WHOLE broadcast (recording start to the live edge), not just the seconds
 * since the viewer joined. It replaces Plyr's session-relative progress bar
 * inside Plyr's own control row, styled through the same Plyr variables the
 * theme already sets (track height, fill gradient, thumb, tooltip), so it
 * reads as the player's own bar.
 *
 * Presentation only. The timeline anchor (`recordedAt`, the broadcast time
 * at recording position 0) comes from Rust, the live/recording switch is the
 * store's, and every decision about where a drag lands is delegated:
 *
 * - inside what the live buffer holds -> `onSeekLive(sessionSeconds)`
 * - at the live edge -> `onGoLive()`
 * - anywhere else -> `onRewindTo(broadcastSeconds)` (Rust swaps the relay
 *   onto the recording at that position)
 * - while rewound: inside the recording -> a plain seek; past its tail
 *   (the last ~40 s before live are not recorded yet) -> `onReturnToLive()`
 *
 * The bar is not linear: `broadcastScale` splits it into five equal sections
 * at 1x, 2x, 4x, 8x and 16x time compression from right to left, so the
 * stretch just before live is fine-grained and the early hours share the
 * far left. Tick marks sit on the section boundaries (labels on hover).
 *
 * Hovering shows the broadcast time under the pointer and how long ago that
 * was. Samples at 4 Hz only while `visible` (the control bar is shown), so a
 * stream watched with the controls hidden pays nothing for it.
 */

export interface BroadcastTimelineProps {
  videoRef: RefObject<HTMLVideoElement | null>;
  /** Element to portal into: Plyr's `.plyr__progress`. */
  host: HTMLElement | null;
  /** Broadcast time at recording position 0 (ISO). */
  anchorIso: string;
  /** Playing the recording (rewound) rather than the live edge. */
  rewound: boolean;
  /** Seconds behind live the LIVE player can still serve by seeking alone.
   *  Small on the low-latency path (its watchdog snaps anything further). */
  liveSeekWindowSecs: number;
  visible: boolean;
  onSeekLive: (sessionSeconds: number) => void;
  onGoLive: () => void;
  onRewindTo: (broadcastSeconds: number) => void;
  onReturnToLive: () => void;
}

/** Dragging this close to the live edge means "go live". */
const LIVE_SNAP_SECS = 8;
/** The recording's tail runs this far behind live; a drag into that gap
 *  cannot be served by the recording and goes live instead. */
const RECORDING_TAIL_GAP_SECS = 45;

function BroadcastTimelineBar({
  videoRef,
  anchorIso,
  rewound,
  liveSeekWindowSecs,
  visible,
  onSeekLive,
  onGoLive,
  onRewindTo,
  onReturnToLive,
}: Omit<BroadcastTimelineProps, 'host'>) {
  const anchorMs = Date.parse(anchorIso);
  const barRef = useRef<HTMLDivElement | null>(null);
  const [state, setState] = useState({ total: 0, pos: 0, tail: 0 });
  const liveEdge = useRef(createLiveEdgeTracker());
  const [drag, setDrag] = useState<number | null>(null);
  const [hover, setHover] = useState<number | null>(null);

  // Live position: broadcast elapsed minus how far the playhead sits behind
  // the freshest buffered edge. Rewound: the recording's own timeline IS the
  // broadcast timeline (position 0 = recordedAt).
  const sample = useCallback(() => {
    const v = videoRef.current;
    if (!v || !Number.isFinite(anchorMs)) return;
    const total = Math.max(1, (Date.now() - anchorMs) / 1000);
    let next;
    if (rewound) {
      const tail = Number.isFinite(v.duration) ? v.duration : total - RECORDING_TAIL_GAP_SECS;
      next = { total, pos: Math.min(v.currentTime, total), tail: Math.min(tail, total) };
    } else {
      // Smoothed: the raw distance to the buffered end is a sawtooth swinging
      // by a whole segment, and `pos` is derived straight from it, so the
      // playhead marker slid backwards and forwards by seconds on a healthy
      // stream. The 0.5 s deadband below cannot absorb a 2-6 s swing.
      const behind = liveEdge.current.behind(v);
      next = { total, pos: Math.max(0, total - behind), tail: total };
    }
    // Paused or at a steady live edge, nothing moved by a visible amount:
    // keep the previous object so React skips the render.
    setState((prev) =>
      Math.abs(prev.pos - next.pos) < 0.5 && Math.abs(prev.total - next.total) < 0.5 && Math.abs(prev.tail - next.tail) < 0.5
        ? prev
        : next,
    );
  }, [videoRef, anchorMs, rewound]);

  // Switching between the live edge and the recording swaps the media
  // timeline underneath us, so the peak window's history describes a stream
  // that is no longer playing. The tracker detects most discontinuities on its
  // own; this is the one we are told about, so say it outright.
  useEffect(() => {
    liveEdge.current.reset();
  }, [rewound]);

  useEffect(() => {
    if (!visible) return;
    const id = window.setInterval(sample, 250);
    return () => window.clearInterval(id);
  }, [visible, sample]);

  const fracFromEvent = (e: React.PointerEvent) => {
    const el = barRef.current;
    if (!el) return 0;
    const r = el.getBoundingClientRect();
    if (r.width <= 0) return 0;
    return Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
  };

  const commit = useCallback(
    (frac: number) => {
      const v = videoRef.current;
      const { total, pos, tail } = state;
      if (!v || total <= 0) return;
      const target = broadcastScale(total).toSecs(frac);
      const behind = total - target;
      // The recording cannot serve its last stretch (its tail chases live and
      // seeking there just stalls), so while rewound the whole tail gap is
      // "go live"; at the live edge itself a small snap zone is enough.
      if (behind <= (rewound ? RECORDING_TAIL_GAP_SECS : LIVE_SNAP_SECS)) {
        if (rewound) onReturnToLive();
        else onGoLive();
        return;
      }
      if (rewound) {
        if (target >= tail - 3) onReturnToLive();
        else v.currentTime = target;
        return;
      }
      // Live: the buffer may already hold this moment.
      const b = v.buffered;
      const held = b.length > 0 ? b.start(0) : v.currentTime;
      const sessionTarget = v.currentTime - (pos - target);
      if (behind <= liveSeekWindowSecs && sessionTarget >= held) {
        onSeekLive(sessionTarget);
        return;
      }
      onRewindTo(target);
    },
    [videoRef, state, rewound, liveSeekWindowSecs, onSeekLive, onGoLive, onRewindTo, onReturnToLive],
  );

  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();
    e.currentTarget.setPointerCapture(e.pointerId);
    setDrag(fracFromEvent(e));
  };
  const onPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const f = fracFromEvent(e);
    if (drag != null) setDrag(f);
    else setHover(f);
  };
  const onPointerUp = (e: React.PointerEvent<HTMLDivElement>) => {
    if (drag == null) return;
    const f = fracFromEvent(e);
    setDrag(null);
    commit(f);
  };

  const { total, pos, tail } = state;
  const scale = broadcastScale(total);
  const posFrac = total > 0 ? scale.toFrac(pos) : 0;
  const tailFrac = total > 0 ? scale.toFrac(tail) : 1;
  const shownFrac = drag ?? posFrac;
  const tipFrac = drag ?? hover;
  let tip: string | null = null;
  if (tipFrac != null && total > 0) {
    const at = scale.toSecs(tipFrac);
    const behind = total - at;
    const liveZone = rewound ? RECORDING_TAIL_GAP_SECS : LIVE_SNAP_SECS;
    tip = behind <= liveZone ? 'LIVE' : `${formatVodTime(at)} · ${formatAgo(behind)}`;
  }

  return (
    <div
      ref={barRef}
      className={`sn-timeline${drag != null ? ' sn-timeline--dragging' : ''}`}
      role="slider"
      aria-label="Broadcast timeline"
      aria-valuemin={0}
      aria-valuemax={Math.round(total)}
      aria-valuenow={Math.round(pos)}
      aria-valuetext={formatVodTime(pos)}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={() => setDrag(null)}
      onPointerLeave={() => setHover(null)}
    >
      <div className="sn-timeline__track">
        {rewound && <div className="sn-timeline__available" style={{ width: `${tailFrac * 100}%` }} />}
        <div className="sn-timeline__fill" style={{ width: `${shownFrac * 100}%` }} />
        {/* Alternating shade over every other compression section, drawn
            above the fill so the scale change shows through the gradient. */}
        {total > 0 &&
          scale.bands.map((band, i) => (
            <div
              key={band.fromFrac}
              className={`sn-timeline__band${i % 2 ? ' sn-timeline__band--alt' : ''}`}
              style={{ left: `${band.fromFrac * 100}%`, width: `${(band.toFrac - band.fromFrac) * 100}%` }}
            />
          ))}
      </div>
      {total > 0 &&
        scale.ticks.map((tick) => (
          <div key={tick.frac} className="sn-timeline__tick" style={{ left: `${tick.frac * 100}%` }}>
            <span className="sn-timeline__tick-label">-{formatBehindShort(tick.behindSecs)}</span>
          </div>
        ))}
      <div className="sn-timeline__thumb" style={{ left: `${shownFrac * 100}%` }} />
      {tip && (
        <div className="sn-timeline__tip" style={{ left: `${(tipFrac ?? 0) * 100}%` }}>
          {tip}
        </div>
      )}
    </div>
  );
}

export default function BroadcastTimeline(props: BroadcastTimelineProps) {
  const { host, ...rest } = props;
  // Mark the host so the stylesheet hides Plyr's own range + buffer bar.
  useEffect(() => {
    if (!host) return;
    host.classList.add('sn-timeline-host');
    return () => host.classList.remove('sn-timeline-host');
  }, [host]);
  if (!host) return null;
  return createPortal(<BroadcastTimelineBar {...rest} />, host);
}
