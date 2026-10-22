// Fine control for the seek bars. The broadcast timeline's compressed scale
// makes a pixel worth more time the further left it sits (16x at the far
// left), and on a long VOD's linear bar a pixel is tens of seconds. Two
// gestures give back precision without changing the scale:
//
// - Slow scrub: while dragging, pulling the pointer away from the bar
//   vertically slows the horizontal mapping. Near the bar a pixel is a
//   pixel; past `FINE_PX` it counts for a quarter; past `FINEST_PX` for a
//   sixteenth, which turns the coarsest section into the finest one's
//   precision.
// - Wheel nudge: a scroll notch over the bar moves the target by a fixed
//   number of seconds, whatever section it is in.

import type { BroadcastScale } from './broadcastScale';

/** A wheel nudge settles (lands, or hands the tooltip back to the pointer)
 *  this long after the last notch. */
export const NUDGE_SETTLE_MS = 600;

/** Vertical distance (px from the bar's centre line) past which a drag slows. */
export const FINE_PX = 28;
export const FINEST_PX = 84;

/** How many pointer pixels one bar pixel costs at this distance. */
export function dragDivisor(distancePx: number): 1 | 4 | 16 {
  if (!(distancePx >= FINE_PX)) return 1;
  return distancePx >= FINEST_PX ? 16 : 4;
}

/** Tooltip suffix while a drag is slowed; empty at full speed. */
export function dragSpeedLabel(divisor: number): string {
  if (divisor >= 16) return '1/16 speed';
  if (divisor >= 4) return '1/4 speed';
  return '';
}

export interface WheelLike {
  deltaX: number;
  deltaY: number;
  shiftKey: boolean;
  ctrlKey: boolean;
}

/** The bar fraction a drag reaches after the pointer moves `dxPx`, at
 *  `divisor` pointer pixels per bar pixel, clamped to the bar. A drag is
 *  integrated from these deltas rather than read off the pointer, so once
 *  slowed the thumb stops sitting under the pointer. */
export function dragFrac(fromFrac: number, dxPx: number, widthPx: number, divisor: number): number {
  if (!(widthPx > 0)) return fromFrac;
  return Math.min(1, Math.max(0, fromFrac + dxPx / widthPx / divisor));
}

/** Seconds a wheel event nudges by: +forward / -back, 0 for a null event.
 *  Wheel up (or right) goes forward in time, Shift makes it a minute, Ctrl a
 *  second. One notch is one step whatever the device's delta size.
 *  Chromium turns Shift + wheel into a horizontal scroll (wheel up arrives as
 *  left), so with Shift held and no vertical delta the horizontal one is read
 *  as the wheel it came from; otherwise Shift + wheel up went back a minute. */
export function wheelStepSecs(e: WheelLike): number {
  const primary =
    e.shiftKey && e.deltaY === 0
      ? -e.deltaX
      : Math.abs(e.deltaX) > Math.abs(e.deltaY)
        ? e.deltaX
        : -e.deltaY;
  if (!primary) return 0;
  const size = e.shiftKey ? 60 : e.ctrlKey ? 1 : 10;
  return Math.sign(primary) * size;
}

/** The bar fraction reached by moving `stepSecs` from `fromFrac`, clamped to
 *  the broadcast. The step is applied in seconds, so it is the same size in
 *  every section of the compressed scale. */
export function nudgeFrac(scale: BroadcastScale, totalSecs: number, fromFrac: number, stepSecs: number): number {
  const at = scale.toSecs(fromFrac) + stepSecs;
  return scale.toFrac(Math.min(totalSecs, Math.max(0, at)));
}
