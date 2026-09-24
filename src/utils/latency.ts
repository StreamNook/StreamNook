// Shared live-latency constants and the automatic live-edge gap.

/**
 * Seconds the parts origin's playlist edge trails the broadcaster, as heard:
 * measured by cross-correlating the audio envelope of the app against
 * twitch.tv playing the same channel on the same machine. The site rode a
 * 1.5 s buffer at what its stats panel called 2.0 s; the app, at an edge
 * distance of 2.7 s, was 0.99 s behind it by ear. So "behind live" on the
 * parts tier is hls.latency plus this, and the governor targets the
 * viewer's gap in those terms. The programme date is NOT used for this: it
 * is stamped on the broadcaster's side and one channel ran 26 s off real
 * time, which had a programme-date target race the buffer to zero.
 */
export const LL_EDGE_DELAY = 0.3;

/** Seconds behind the broadcaster on the parts tier, from the edge distance. */
export function behindLiveFromEdge(hlsLatency: number): number {
  return Math.max(0, hlsLatency + LL_EDGE_DELAY);
}

/** The edge distance (hls.latency) that lands a gap stated in seconds behind live. */
export function edgeTargetForGap(gapSecs: number): number {
  return Math.max(0, gapSecs - LL_EDGE_DELAY);
}

/**
 * Which delivery the live player is riding. Decided once per stream start from
 * two relay facts: whether the parts origin took the stream over, and whether the
 * upstream carries Twitch's PREFETCH hints (a low-latency broadcast).
 *
 * - `ll`: the parts origin serves a spec LL-HLS playlist; hls.js runs in
 *   lowLatencyMode and delivery pauses no more than a few hundred ms.
 * - `promotion`: a low-latency broadcast ridden on whole segments (the engine
 *   is off or cannot take this container); the relay promotes the in-progress
 *   segments, a tighter but thinner ride.
 * - `plain`: a normal-latency broadcast, whole 2 s segments with real delivery
 *   jitter; the honest floor is around 5 s.
 */
export type LivePath = 'll' | 'promotion' | 'plain';

/**
 * The live-edge gap (seconds behind the broadcaster) used when the viewer has
 * not set one.
 * Per path, because one number cannot serve all three: measured 2026-09-21,
 * the parts origin rode 2.0 with a 2.7 s minimum forward buffer against
 * delivery gaps of at most 0.34 s, a promoted H.264 broadcast at 2.0 ran its
 * forward buffer down to 0.15 s, and normal-latency broadcasts stall below
 * about 4 (June: cushion 3 stalled, 4 held at twitch.tv parity). The
 * stall-adaptive cushion still ramps any of these up on a channel that
 * cannot sustain it.
 */
export const AUTO_GAP: Record<LivePath, number> = {
  // twitch.tv rides a 1.5 to 1.8 s buffer at what its panel calls 2.0 to
  // 2.3 s; 2.0 here lands an edge distance of 1.7 s, level with the site by
  // ear, and the stall-adaptive cushion still ramps a channel that cannot
  // sustain it.
  ll: 2.0,
  promotion: 3.0,
  plain: 5.0,
};

/** The gap a session should target: the viewer's own, else the path's automatic one. */
export function resolveLiveEdgeGap(setting: number | null | undefined, path: LivePath): number {
  return typeof setting === 'number' && Number.isFinite(setting) && setting > 0 ? setting : AUTO_GAP[path];
}

/** What the gap slider shows while the setting is automatic. */
export const LL_TARGET_DEFAULT = AUTO_GAP.ll;
