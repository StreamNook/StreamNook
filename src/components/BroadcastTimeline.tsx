import { useCallback, useEffect, useRef, useState, type RefObject } from 'react';
import { createPortal } from 'react-dom';
import { formatAgo, formatVodTime } from '../utils/vodProgress';
import { broadcastScale, formatBehindShort } from '../utils/broadcastScale';
import { createLiveEdgeTracker } from '../utils/liveEdge';
import { dragDivisor, dragSpeedLabel, nudgeFrac, wheelStepSecs } from '../utils/scrubPrecision';

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
 *
 * A drop is held where it landed until the sampled position agrees: the
 * seek (or the relay swap onto the recording) takes anything from a frame
 * to a couple of seconds, and without the hold the thumb jumped back to the
 * old position and then forward again, which read as a stutter on every
 * small drag.
 *
 * Precision on the compressed sections (see `scrubPrecision`): while
 * dragging, pulling the pointer away from the bar slows the horizontal
 * mapping to a quarter and then a sixteenth, so the coarse left end can be
 * scrubbed as finely as the right; and a scroll notch over the bar nudges
 * the target by ten seconds (Shift a minute, Ctrl a second), applied in
 * seconds so it is the same step everywhere. A wheel nudge is shown at once
 * and lands when the wheel goes quiet.
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
   *  Same on both delivery paths: what hls.js tolerates before it seeks
   *  forward by itself (55 s), inside the 30 s back buffer. */
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

/** A drop the bar is still waiting on. Shown at the drop point until the
 *  sampled position agrees or the wait runs out, so the playhead never jumps
 *  back to where it was while the seek (or the relay swap) lands. */
export interface PendingSeek {
  frac: number;
  secs: number;
  at: number;
}
/** The sampled position counts as "arrived" this close to the drop. At the
 *  finest section of the scale that is about one pixel. */
const PENDING_SETTLE_SECS = 3;
/** A drop nothing ever confirms (the swap failed, the seek was refused)
 *  stops being shown after this. */
const PENDING_TIMEOUT_MS = 8000;
/** A wheel nudge lands this long after the last notch. */
const NUDGE_SETTLE_MS = 600;

type BarProps = Omit<BroadcastTimelineProps, 'host'> & {
  pending: PendingSeek | null;
  setPending: (p: PendingSeek | null) => void;
};

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
  pending,
  setPending,
}: BarProps) {
  const anchorMs = Date.parse(anchorIso);
  const barRef = useRef<HTMLDivElement | null>(null);
  const [state, setState] = useState({ total: 0, pos: 0, tail: 0 });
  const liveEdge = useRef(createLiveEdgeTracker());
  const [drag, setDrag] = useState<number | null>(null);
  // How many pointer pixels a bar pixel costs right now (1, 4 or 16), for
  // the tooltip; the drag itself is integrated in `dragRef`.
  const [dragRate, setDragRate] = useState(1);
  // A drag is integrated from pointer deltas rather than read off the
  // pointer's absolute position, because the slow-scrub gesture makes the
  // thumb move less than the pointer.
  const dragRef = useRef<{ frac: number; lastX: number } | null>(null);
  const [hover, setHover] = useState<number | null>(null);
  // A wheel nudge waiting to land: shown as the thumb and tooltip until the
  // wheel has been quiet for NUDGE_SETTLE_MS, then committed like a drop.
  const [nudge, setNudge] = useState<number | null>(null);
  const nudgeRef = useRef<number | null>(null);
  const nudgeTimerRef = useRef<number | null>(null);
  // The playhead as of the last sample, so a seek can be measured (see the
  // `seeking` listener below). Refreshed right before the bar's own seeks so
  // those measure exactly.
  const lastCtRef = useRef(0);
  // The bar's box for the current hover or drag session. Read once on entry
  // rather than per pointer move: a move must never force layout. The
  // track's hover growth changes height only, never left or width.
  const rectRef = useRef<DOMRect | null>(null);

  // Live position: broadcast elapsed minus how far the playhead sits behind
  // the freshest buffered edge. Rewound: the recording's own timeline IS the
  // broadcast timeline (position 0 = recordedAt).
  const sample = useCallback(() => {
    const v = videoRef.current;
    if (!v || !Number.isFinite(anchorMs)) return;
    const total = Math.max(1, (Date.now() - anchorMs) / 1000);
    let next;
    // Only a real reading can confirm a drop: while the rebuilt element has
    // no metadata the pending position stands in for the playhead, and that
    // must not count as the playhead having arrived.
    let measured = true;
    if (rewound) {
      // A freshly rebuilt element reports no duration and a 0 playhead until
      // its metadata lands; the drop it is loading is the truth until then.
      const ready = Number.isFinite(v.duration) && v.duration > 0;
      measured = ready;
      const ct = ready ? v.currentTime : (pending?.secs ?? v.currentTime);
      const tail = ready ? v.duration : total - RECORDING_TAIL_GAP_SECS;
      next = { total, pos: Math.min(ct, total), tail: Math.min(tail, total) };
    } else {
      // Smoothed: the raw distance to the buffered end is a sawtooth swinging
      // by a whole segment, and `pos` is derived straight from it, so the
      // playhead marker slid backwards and forwards by seconds on a healthy
      // stream. The 0.5 s deadband below cannot absorb a 2-6 s swing.
      const behind = liveEdge.current.behind(v);
      next = { total, pos: Math.max(0, total - behind), tail: total };
    }
    if (!v.seeking) lastCtRef.current = v.currentTime;
    if (
      pending &&
      ((measured && Math.abs(next.pos - pending.secs) <= PENDING_SETTLE_SECS) || Date.now() - pending.at > PENDING_TIMEOUT_MS)
    ) {
      setPending(null);
    }
    // Paused or at a steady live edge, nothing moved by a visible amount:
    // keep the previous object so React skips the render.
    setState((prev) =>
      Math.abs(prev.pos - next.pos) < 0.5 && Math.abs(prev.total - next.total) < 0.5 && Math.abs(prev.tail - next.tail) < 0.5
        ? prev
        : next,
    );
  }, [videoRef, anchorMs, rewound, pending, setPending]);

  // Switching between the live edge and the recording swaps the media
  // timeline underneath us, so the peak window's history describes a stream
  // that is no longer playing. The tracker detects most discontinuities on its
  // own; this is the one we are told about, so say it outright.
  useEffect(() => {
    liveEdge.current.reset();
  }, [rewound]);

  // Sample at once as well: the bar remounts under a rebuilt control bar
  // after every relay swap, and an empty first paint (no bands, thumb at the
  // far left) for a quarter second read as a flicker on every rewind.
  useEffect(() => {
    if (!visible) return;
    sample();
    const id = window.setInterval(sample, 250);
    return () => window.clearInterval(id);
  }, [visible, sample]);

  // Every seek re-bases the smoothing window by exactly the jump, whoever
  // made it (this bar, the keyboard, Go Live): a small forward seek is under
  // the window's own discontinuity threshold, so without this it kept
  // reporting the pre-seek distance for a full window and the thumb lagged
  // the drop by up to six seconds.
  useEffect(() => {
    const v = videoRef.current;
    if (!v) return;
    const onSeeking = () => {
      const from = lastCtRef.current;
      const to = v.currentTime;
      if (from > 0) liveEdge.current.shift(to - from);
      lastCtRef.current = to;
    };
    v.addEventListener('seeking', onSeeking);
    return () => v.removeEventListener('seeking', onSeeking);
  }, [videoRef]);

  const fracFromEvent = (e: React.PointerEvent) => {
    const el = barRef.current;
    if (!el) return 0;
    const r = rectRef.current ?? el.getBoundingClientRect();
    if (r.width <= 0) return 0;
    return Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
  };

  /** Lands a drop. Returns the broadcast second it aimed at, or null when the
   *  drop meant "go live" (the thumb is at the edge already, nothing to hold). */
  const commit = useCallback(
    (frac: number): number | null => {
      const v = videoRef.current;
      const { total, pos, tail } = state;
      if (!v || total <= 0) return null;
      const target = broadcastScale(total).toSecs(frac);
      const behind = total - target;
      // The recording cannot serve its last stretch (its tail chases live and
      // seeking there just stalls), so while rewound the whole tail gap is
      // "go live"; at the live edge itself a small snap zone is enough.
      if (behind <= (rewound ? RECORDING_TAIL_GAP_SECS : LIVE_SNAP_SECS)) {
        if (rewound) onReturnToLive();
        else onGoLive();
        return null;
      }
      if (rewound) {
        if (target >= tail - 3) {
          onReturnToLive();
          return null;
        }
        lastCtRef.current = v.currentTime;
        v.currentTime = target;
        return target;
      }
      // Live: the buffer may already hold this moment.
      const b = v.buffered;
      const held = b.length > 0 ? b.start(0) : v.currentTime;
      const sessionTarget = v.currentTime - (pos - target);
      if (behind <= liveSeekWindowSecs && sessionTarget >= held) {
        lastCtRef.current = v.currentTime;
        onSeekLive(sessionTarget);
        return target;
      }
      onRewindTo(target);
      return target;
    },
    [videoRef, state, rewound, liveSeekWindowSecs, onSeekLive, onGoLive, onRewindTo, onReturnToLive],
  );

  // The latest commit, for the wheel listener below (a native listener,
  // registered once, must not close over a stale `commit`).
  const commitRef = useRef(commit);
  const stateRef = useRef(state);
  const pendingRef = useRef(pending);
  useEffect(() => {
    commitRef.current = commit;
    stateRef.current = state;
    pendingRef.current = pending;
  });

  const clearNudge = useCallback(() => {
    if (nudgeTimerRef.current != null) {
      window.clearTimeout(nudgeTimerRef.current);
      nudgeTimerRef.current = null;
    }
    nudgeRef.current = null;
    setNudge(null);
  }, []);

  const onPointerEnter = () => {
    rectRef.current = barRef.current?.getBoundingClientRect() ?? null;
  };
  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();
    e.currentTarget.setPointerCapture(e.pointerId);
    rectRef.current = e.currentTarget.getBoundingClientRect();
    clearNudge();
    setPending(null);
    const f = fracFromEvent(e);
    dragRef.current = { frac: f, lastX: e.clientX };
    setDragRate(1);
    setDrag(f);
  };
  const onPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const d = dragRef.current;
    if (d == null) {
      setHover(fracFromEvent(e));
      return;
    }
    const r = rectRef.current;
    if (!r || r.width <= 0) return;
    // Slow scrub: the further the pointer sits above or below the bar, the
    // less of its horizontal motion reaches the thumb.
    const divisor = dragDivisor(Math.abs(e.clientY - (r.top + r.height / 2)));
    const dx = e.clientX - d.lastX;
    d.lastX = e.clientX;
    d.frac = Math.min(1, Math.max(0, d.frac + dx / r.width / divisor));
    setDragRate(divisor);
    setDrag(d.frac);
  };
  const onPointerUp = () => {
    const d = dragRef.current;
    if (d == null) return;
    dragRef.current = null;
    const f = d.frac;
    setDrag(null);
    setDragRate(1);
    const target = commit(f);
    setPending(target == null ? null : { frac: f, secs: target, at: Date.now() });
  };
  const onPointerCancel = () => {
    dragRef.current = null;
    setDrag(null);
    setDragRate(1);
  };
  const onPointerLeave = () => {
    rectRef.current = null;
    setHover(null);
  };

  // Wheel nudge. A native listener because React registers `wheel` as
  // passive, and the page must not scroll (or the player change volume) when
  // the wheel is used to seek.
  useEffect(() => {
    const el = barRef.current;
    if (!el) return;
    // A nudge starts from the drop still being held, else from the playhead.
    const parkedFrac = (scale: ReturnType<typeof broadcastScale>, playheadSecs: number) =>
      pendingRef.current ? pendingRef.current.frac : scale.toFrac(playheadSecs);
    const onWheel = (e: WheelEvent) => {
      const step = wheelStepSecs(e);
      if (!step) return;
      e.preventDefault();
      e.stopPropagation();
      const { total, pos } = stateRef.current;
      if (total <= 0) return;
      const scale = broadcastScale(total);
      const d = dragRef.current;
      if (d) {
        // Mid-drag: the wheel fine-tunes the drag itself.
        d.frac = nudgeFrac(scale, total, d.frac, step);
        setDrag(d.frac);
        return;
      }
      const from = nudgeRef.current ?? parkedFrac(scale, pos);
      const next = nudgeFrac(scale, total, from, step);
      nudgeRef.current = next;
      setNudge(next);
      if (nudgeTimerRef.current != null) window.clearTimeout(nudgeTimerRef.current);
      nudgeTimerRef.current = window.setTimeout(() => {
        nudgeTimerRef.current = null;
        const f = nudgeRef.current;
        nudgeRef.current = null;
        setNudge(null);
        if (f == null) return;
        const target = commitRef.current(f);
        setPending(target == null ? null : { frac: f, secs: target, at: Date.now() });
      }, NUDGE_SETTLE_MS);
    };
    el.addEventListener('wheel', onWheel, { passive: false });
    return () => {
      el.removeEventListener('wheel', onWheel);
      if (nudgeTimerRef.current != null) window.clearTimeout(nudgeTimerRef.current);
    };
  }, [setPending]);

  const { total, pos, tail } = state;
  const scale = broadcastScale(total);
  const posFrac = total > 0 ? scale.toFrac(pos) : 0;
  const tailFrac = total > 0 ? scale.toFrac(tail) : 1;
  const shownFrac = drag ?? nudge ?? (pending ? pending.frac : posFrac);
  const tipFrac = drag ?? nudge ?? hover;
  let tip: string | null = null;
  if (tipFrac != null && total > 0) {
    const at = scale.toSecs(tipFrac);
    const behind = total - at;
    const liveZone = rewound ? RECORDING_TAIL_GAP_SECS : LIVE_SNAP_SECS;
    tip = behind <= liveZone ? 'LIVE' : `${formatVodTime(at)} · ${formatAgo(behind)}`;
    const speed = drag != null ? dragSpeedLabel(dragRate) : '';
    if (speed) tip = `${tip} · ${speed}`;
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
      data-no-wheel-volume
      onPointerEnter={onPointerEnter}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerCancel}
      onPointerLeave={onPointerLeave}
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

/** The drop that caused a relay swap, parked between player instances: the
 *  solo player is keyed by its stream URL, so the swap remounts it and every
 *  piece of React state under it, and the thumb flashed to wherever the fresh
 *  element reported (0) until the recording loaded. Keyed by the broadcast
 *  anchor so a drop never leaks onto another broadcast, and expires with the
 *  same timeout the bar applies. */
let parkedDrop: { anchorIso: string; drop: PendingSeek } | null = null;

export default function BroadcastTimeline(props: BroadcastTimelineProps) {
  const { host, anchorIso, ...rest } = props;
  const [pending, setPendingState] = useState<PendingSeek | null>(() => {
    const parked = parkedDrop;
    if (!parked || parked.anchorIso !== anchorIso || Date.now() - parked.drop.at > PENDING_TIMEOUT_MS) return null;
    return parked.drop;
  });
  const setPending = useCallback(
    (p: PendingSeek | null) => {
      parkedDrop = p ? { anchorIso, drop: p } : null;
      setPendingState(p);
    },
    [anchorIso],
  );
  // Mark the host so the stylesheet hides Plyr's own range + buffer bar.
  useEffect(() => {
    if (!host) return;
    host.classList.add('sn-timeline-host');
    return () => host.classList.remove('sn-timeline-host');
  }, [host]);
  if (!host) return null;
  return createPortal(<BroadcastTimelineBar {...rest} anchorIso={anchorIso} pending={pending} setPending={setPending} />, host);
}
