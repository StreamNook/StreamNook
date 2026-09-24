import type Hls from 'hls.js';

/**
 * Continuous live-latency maintenance, shared by the solo player and MultiNook tiles.
 *
 * hls.js runs here with `lowLatencyMode:false` (the correct setting for Twitch over a
 * proxy: native LL-HLS chunk parsing causes cyclic starvation). A side effect is that
 * hls.js's OWN playback-rate catch-up is gated off, so nothing holds the playhead at
 * the configured cushion after the cold-start snap and latency drifts upward across a
 * session (the "10-20s behind live" bug).
 *
 * This governor restores that maintenance WITHOUT ever seeking (seeking a live hls.js
 * stream toward the edge mid-playback freezes it — a hard-won lesson). It nudges
 * `video.playbackRate` only.
 *
 * WHAT IT MEASURES — the FORWARD BUFFER (`bufferedEnd - currentTime`), NOT the
 * distance to the playlist's live edge. This distinction is load-bearing: on a
 * low-latency channel the relay promotes Twitch's in-progress PREFETCH segments, so
 * hls.js's reported live edge sits ahead of what is actually downloadable. Chasing
 * that edge makes the governor accelerate into the in-progress zone and starve the
 * buffer (constant stalls). The forward buffer is always reachable, so targeting it
 * is safe: the governor speeds up ONLY when there is EXCESS forward buffer (the
 * playhead has fallen behind and there is downloaded content to consume), and that
 * very act shrinks the excess back to the target — it can never drain the buffer
 * below the target, so it cannot cause a stall.
 *
 * User speed control is respected for free: every Plyr speed-up option is >= 1.25
 * (above any sane `ceiling`) and every slow-mo option is < 1.0, so if the current
 * playback rate sits outside (1.0 .. ceiling] the user has taken manual control and
 * the governor stands down until the rate returns to 1.0.
 */
export interface LatencyGovernorOptions {
  /** Target forward-buffer seconds. Defaults to reading `hls.config.liveSyncDuration` (the cushion). */
  getTarget?: () => number;
  /** Max rate the governor will use to catch up. Keep < 1.25 (Plyr's lowest speed-up) so user selections are never fought. */
  ceiling?: number;
  /**
   * Seconds of forward buffer ABOVE target before the governor starts catching up.
   *
   * Must clear one whole segment on segment-delivered streams. Delivery adds a
   * segment at a time, so a band narrower than that is overshot on every single
   * arrival and the rate oscillates instead of ever settling. Pass a getter when
   * the real segment length is only known once a playlist has landed.
   */
  band?: number | (() => number);
  /** Poll interval (ms). */
  tickMs?: number;
  /** Forward buffer beyond `target + dvrSlack` is treated as a deliberate DVR scrub-back and left alone. */
  dvrSlack?: number;
  /**
   * Max playbackRate change per tick. When set, the rate RAMPS toward its
   * computed value instead of stepping to it, in both directions. Abrupt rate
   * steps are audible through the pitch corrector (a pop or warble, obvious on
   * music) and read as a micro-hitch; a slide of ~0.01/tick is imperceptible.
   * Unset = legacy stepping (set the computed rate directly).
   */
  rampStep?: number | (() => number);
  /**
   * Low-buffer protection: when the forward buffer falls BELOW this (seconds),
   * ease the rate down toward `slowRate` so the playhead stops outrunning a
   * draining buffer. On the low-latency path the forward buffer cannot exceed
   * the distance behind live, so margins are inherently thin (~1.5s) and a
   * delivery wobble of a few hundred ms otherwise drains to a hard stall; a
   * 3% slowdown buys ~30ms of margin per second, exactly the class of stall
   * that misses by hairs. Unset = no slow side (legacy behavior).
   */
  floor?: number;
  /** Minimum rate used for low-buffer protection. Keep above Plyr's slow-mo
   *  options (<= 0.75) so user selections are still recognized as manual. */
  slowRate?: number;
  /**
   * Seconds of forward buffer ABOVE `floor` at which the full `ceiling` becomes
   * available. Overspeed consumes the forward buffer, so the allowed rate scales
   * with the headroom actually there to consume: 1.0 at the floor, the full
   * ceiling at floor + engageSpan, linear between. Without this, a catch-up
   * signal (behind-live or buffer excess) pins the rate at the ceiling while
   * the buffer sits barely above the floor, drains it into the floor regime,
   * eases down, refills, and seesaws forever: the rate never settles (audible
   * as continuous pitch-corrector crackle) and the drain regularly overshoots
   * into a hard stall. Only meaningful when `floor` is set. Default 1.5.
   */
  engageSpan?: number;
  /**
   * Behind-live target in seconds. When set (with `getLatency`), rate control is
   * driven by behind-live distance, not forward-buffer excess, and works in BOTH
   * directions: the governor speeds up to pull the PLAYHEAD closer to live when
   * it falls behind the target, and slows down (toward `slowRate`) to let the gap
   * grow back when it drifts ahead of the target — holding the playhead near
   * ~this value. The buffer stays full because the origin refills it from the edge
   * as fast as it's consumed, and easing back only grows it; the low-buffer
   * `floor` still takes precedence on the SLOW side, so neither direction can
   * drain the buffer into a stall. Only valid on the LL-origin path, where
   * `hls.latency` is honest (only real parts are listed). Pass a getter (not a
   * fixed number) so a mid-stream change to the viewer's chosen gap takes effect
   * immediately, without rebuilding the player.
   */
  latencyTarget?: number | (() => number);
  /** Current behind-live seconds (e.g. `() => hls.latency`). Paired with `latencyTarget`. */
  getLatency?: () => number | null;
  /**
   * Latency-targeting only: how close to the target (seconds) a catch-up
   * runs before it lets go. The band decides when to ENGAGE; this decides
   * when to RELEASE. Without it the governor released the moment the excess
   * was back inside the band, so every session that started behind (a cold
   * start lands past the cushion) settled at target + band and stayed there.
   * The two thresholds are the hysteresis: jitter inside the band never
   * re-engages, a real drift still does, and a catch-up once begun runs to
   * the target. Inside the band the catch-up runs at a fixed `finishRate`
   * rather than a proportional glide, so the audible part is one step up,
   * a hold, one step down: a glide wrote a new rate every tick for the
   * whole approach, and each write is a pitch-corrector artifact. The slow
   * side does not extend: too close to the edge is corrected only past
   * -band and let go inside it, because a slowed, pitch-corrected stream is
   * what a viewer hears as distortion and the buffer there is ample anyway.
   * Default 0.15.
   */
  release?: number;
  /** Rate used to finish a catch-up inside the band. Default 1.03: audible
   *  through the pitch corrector only as a slight brightness, unlike 1.05. */
  finishRate?: number;
  /**
   * Catch-up gain: rate increase per second of excess past the band. Default
   * 0.03 suits multi-second drift recovery, but it's far too weak for holding
   * a tight latency target — at 0.3s over it yields 1.003x (invisible, and
   * below the apply threshold, so it never engages). Latency targeting wants a
   * steeper gain (~0.12) so even a few hundred ms over produces real catch-up.
   */
  gain?: number;
  /** Optional label for logs. */
  label?: string;
  /** Optional debug logger (callers pass their own to avoid a logger dependency here). */
  log?: (msg: string) => void;
}

/** Fallback band, used until a stream reports its real segment length. */
export const DEFAULT_LATENCY_BAND = 1.5;

const DEFAULTS = {
  ceiling: 1.05,
  band: DEFAULT_LATENCY_BAND,
  tickMs: 2000,
  dvrSlack: 25,
};

function forwardBuffer(video: HTMLVideoElement): number {
  try {
    const b = video.buffered;
    return b.length > 0 ? Math.max(0, b.end(b.length - 1) - video.currentTime) : 0;
  } catch {
    // buffered can throw if the element is mid-teardown.
    return 0;
  }
}

/**
 * Start the governor against a live hls.js instance. Returns a stop function that
 * clears the loop and restores normal playback rate. Safe to call the stop function
 * more than once.
 */
export function startLatencyGovernor(
  hls: Hls,
  video: HTMLVideoElement,
  options: LatencyGovernorOptions = {},
): () => void {
  const ceiling = options.ceiling ?? DEFAULTS.ceiling;
  const bandOf = () =>
    typeof options.band === 'function' ? options.band() : (options.band ?? DEFAULTS.band);
  const tickMs = options.tickMs ?? DEFAULTS.tickMs;
  const dvrSlack = options.dvrSlack ?? DEFAULTS.dvrSlack;
  const floor = options.floor;
  const slowRate = options.slowRate ?? 0.97;
  const engageSpan = options.engageSpan ?? 1.5;
  // The lowest rate this governor will ever set itself; anything below it is
  // a manual user speed selection and must not be fought. Both the low-buffer
  // floor and the latency-target slow side (below) can ease the rate down to
  // slowRate, so either one makes slowRate the governor-owned minimum.
  const lowestOwned = floor != null || options.latencyTarget != null ? slowRate : 1.0;
  const release = options.release ?? 0.15;
  const finishRate = options.finishRate ?? 1.03;
  // Latency-targeting hysteresis: a catch-up in progress (see `release`).
  let catchingUp = false;
  const getTarget =
    options.getTarget ??
    (() => {
      const t = hls.config.liveSyncDuration;
      return typeof t === 'number' && Number.isFinite(t) ? t : 6;
    });

  const resetRate = () => {
    const r = video.playbackRate;
    if (r !== 1.0 && r >= lowestOwned - 0.001 && r <= ceiling) {
      video.playbackRate = 1.0;
    }
  };

  const tick = () => {
    if (video.paused || video.seeking) return;

    // Hand control back to the user if they've chosen a speed outside our band.
    const rate = video.playbackRate;
    if (rate < lowestOwned - 0.005 || rate > ceiling) return;

    const target = getTarget();
    const band = bandOf();
    const fb = forwardBuffer(video);

    // A very large forward buffer means the user scrubbed back into the DVR window;
    // leave it to them (Go Live snaps back to live).
    if (fb > target + dvrSlack) {
      resetRate();
      return;
    }

    // The catch-up signal: behind-live distance when latency-targeting (drives
    // the PLAYHEAD toward live), else forward-buffer excess (legacy). The
    // floor below always governs the slow side on forward buffer, so neither
    // mode can drain the buffer into a stall.
    // Resolve the latency target each tick so a getter reflects the viewer's
    // current setting (a mid-stream gap change applies without a player rebuild).
    const latencyTargetVal =
      typeof options.latencyTarget === 'function'
        ? options.latencyTarget()
        : options.latencyTarget;
    const latencyTargeting = latencyTargetVal != null && Number.isFinite(latencyTargetVal);
    const latency = latencyTargeting ? (options.getLatency?.() ?? null) : null;
    const usingLatency = latencyTargeting && latency != null;
    const excess = usingLatency
      ? (latency as number) - (latencyTargetVal as number)
      : fb - target;

    // Regimes, ramped between so transitions are inaudible:
    //  - below the floor: low-buffer protection — drain slower than delivery so
    //    a wobble is ridden out as a slight slowdown instead of a stall. This is
    //    checked FIRST so catch-up never overrides buffer safety;
    //  - excess above band: speed up proportionally toward the target, capped at
    //    the ceiling;
    //  - excess below -band (latency-targeting only): too close to the edge, so
    //    slow down proportionally to let the gap grow back toward the target,
    //    floored at slowRate. Symmetric with the speed-up side. Safe because
    //    easing the playhead back only ever GROWS the buffer (the origin keeps
    //    refilling from the edge), so it cannot stall — the low-buffer floor
    //    above still takes precedence if delivery is actually thin. Gated to the
    //    latency path: on the forward-buffer path a negative excess just means
    //    the buffer is at/below target, which the floor already handles;
    //  - otherwise: real time.
    const gain = options.gain ?? 0.03;
    // Overspeed only against buffer that is actually there to consume: the
    // ceiling scales from 1.0 at the floor to its full value at
    // floor + engageSpan. A catch-up signal the buffer cannot back (the
    // downloadable edge sits just ahead of the playhead, e.g. the relay itself
    // is the bottleneck) then resolves to 1.0 instead of grinding at the
    // ceiling, draining into the floor, and seesawing between regimes.
    const effCeiling =
      floor != null
        ? 1 + (ceiling - 1) * Math.min(1, Math.max(0, (fb - floor) / engageSpan))
        : ceiling;
    let desired: number;
    if (floor != null && fb < floor) {
      desired = slowRate;
      catchingUp = false;
    } else if (usingLatency) {
      // Engage past the band, run to within `release` of the target, let go.
      if (catchingUp && excess <= release) catchingUp = false;
      if (!catchingUp && excess > band) catchingUp = true;
      if (catchingUp) {
        // Past the band: proportional, as before. Inside it: the fixed
        // finish rate, so the last stretch is a hold, not a glide.
        const proportional = excess > band ? 1 + gain * (excess - band) : 1.0;
        desired = Math.max(1.0, Math.min(effCeiling, Math.max(finishRate, proportional)));
      } else if (excess < -band) {
        desired = Math.min(1.0, Math.max(slowRate, 1 + gain * (excess + band)));
      } else {
        desired = 1.0;
      }
    } else {
      // Forward-buffer mode settles between target and target + band by
      // design: delivery adds a whole segment at a time, so a tighter hold
      // would oscillate on every arrival.
      desired = excess > band ? Math.max(1.0, Math.min(effCeiling, 1 + gain * (excess - band))) : 1.0;
    }
    const rampStep =
      typeof options.rampStep === 'function' ? options.rampStep() : options.rampStep;
    let next = rampStep
      ? rate + Math.max(-rampStep, Math.min(rampStep, desired - rate))
      : desired;
    // Safety beats the audible-comfort ramp: never keep overspeeding a
    // sub-floor buffer while a slow ramp glides down (at 0.01/tick the descent
    // from the ceiling takes many seconds, which is exactly how a wobble
    // becomes a hard stall). One step down to real time is far less audible
    // than the stall it prevents; the ramp still handles 1.0 -> slowRate.
    if (floor != null && fb < floor && next > 1.0) next = 1.0;
    // Real time means exactly 1.0. A residual like 1.004 sat inside the write
    // threshold below and was never cleared, which kept the browser's
    // time-stretcher engaged for most of a session (measured 88% of ticks).
    if (desired === 1.0 && rate !== 1.0 && Math.abs(rate - 1.0) <= 0.0049) next = 1.0;
    if (Math.abs(next - rate) > 0.0049 || (next === 1.0 && rate !== 1.0)) {
      // Round away float dust so repeated ramp arithmetic stays on clean values.
      video.playbackRate = Math.round(next * 1000) / 1000;
      const detail = usingLatency
        ? `behind-live ${(latency as number).toFixed(1)}s (target ${latencyTargetVal}s, buffer ${fb.toFixed(1)}s)`
        : `forward buffer ${fb.toFixed(1)}s (settles ${target.toFixed(1)}-${(target + band).toFixed(1)}s)`;
      options.log?.(
        `[Latency${options.label ? ` ${options.label}` : ''}] ${detail} -> rate ${video.playbackRate.toFixed(3)}`,
      );
    }
  };

  const id = window.setInterval(tick, tickMs);
  let stopped = false;
  return () => {
    if (stopped) return;
    stopped = true;
    window.clearInterval(id);
    try {
      resetRate();
    } catch {
      // element may already be gone
    }
  };
}
