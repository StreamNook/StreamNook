// Zoom to fill on the phones (SN-30): in landscape the picture fills a long
// screen by trimming its top and bottom, never its sides, and never past 1.5x.
// The rules are pure so both phones test them against one case table
// (src/mobile/phoneGestureCases.json; the iPhone's twin is SNKit VideoFill).
// The surface is the one painter of the zoom wrapper's transform.

export const FILL_CAP = 1.5;
const FILL_MIN = 1.01;
export const PINCH_OUT = 1.1;
export const PINCH_IN = 0.9;
const REARM_OUT = 1.05;
const REARM_IN = 0.95;
const PREVIEW_FLOOR = 0.85;
const PREVIEW_OVER = 1.15;

const usable = (n: number) => Number.isFinite(n) && n > 0;

/** How much a `vw` x `vh` video drawn whole in a `bw` x `bh` box is scaled so it
 *  fills the box's width: 1 when the box is no wider than the video (filling
 *  would crop the sides), at most FILL_CAP, and 1 for a difference too small
 *  to see. */
export function fillScale(bw: number, bh: number, vw: number, vh: number): number {
  if (!usable(bw) || !usable(bh) || !usable(vw) || !usable(vh)) return 1;
  const r = bw / bh / (vw / vh);
  if (r <= 1) return 1;
  const s = Math.min(r, FILL_CAP);
  return s < FILL_MIN ? 1 : s;
}

/** The picture's scale under two fingers: the rest it started from times how
 *  far the fingers spread, held a little past both ends. */
export function pinchPreviewScale(startFill: boolean, fill: number, ratio: number): number {
  const base = startFill ? fill : 1;
  const s = base * (Number.isFinite(ratio) ? ratio : 1);
  return Math.min(Math.max(s, PREVIEW_FLOOR), Math.max(fill, 1) * PREVIEW_OVER);
}

/** What a pinch chose: out past 10 percent from Fit fills, in past 10 percent
 *  from Fill fits, anything less keeps the start. Nothing to fill keeps it too. */
export function pinchDecision(startFill: boolean, fill: number, ratio: number): boolean {
  if (fill <= 1) return startFill;
  if (startFill && ratio < PINCH_IN) return false;
  if (!startFill && ratio > PINCH_OUT) return true;
  return startFill;
}

/** One tick when a pinch crosses the line that changes the choice, re-armed only
 *  once the fingers come back past 1.05 (or 0.95), so a resting pinch never
 *  buzzes. Nothing to fill never ticks. */
export function pinchTick(
  armed: boolean,
  startFill: boolean,
  fill: number,
  ratio: number,
): { tick: boolean; armed: boolean } {
  if (fill <= 1) return { tick: false, armed };
  const crossed = startFill ? ratio < PINCH_IN : ratio > PINCH_OUT;
  if (armed && crossed) return { tick: true, armed: false };
  const back = startFill ? ratio > REARM_IN : ratio < REARM_OUT;
  if (!armed && back) return { tick: false, armed: true };
  return { tick: false, armed };
}

/** What the surface paints on: the zoom wrapper (any element with a style). */
export interface ZoomTarget {
  style: { transform: string; transition: string };
}

export interface ZoomSurface {
  /** The player's inputs: whether this layout zooms at all, and the choice. */
  setInputs(next: { zoomable: boolean; want: boolean }): void;
  /** The fill scale for the current box and video. */
  setFill(fill: number): void;
  /** A pinch in flight: this scale, with no transition. */
  preview(scale: number): void;
  /** The pinch is over: back to rest, animated. */
  endPreview(): void;
  isOn(): boolean;
  fillScale(): number;
  dispose(): void;
}

const EASE = 'transform 220ms cubic-bezier(0.2, 0.8, 0.2, 1)';

/** The one painter of `el`'s transform and transition. Only a change of the
 *  choice animates (and only under full motion); a layout change (zoomable,
 *  the fill scale) applies at once and drops any preview. */
export function createZoomSurface(el: ZoomTarget, readMotion: () => string | undefined): ZoomSurface {
  let zoomable = false;
  let want = false;
  let fill = 1;
  let preview: number | null = null;
  let disposed = false;
  const paint = (animate: boolean) => {
    if (disposed) return;
    const s = preview ?? (zoomable && want ? fill : 1);
    const motion = readMotion();
    el.style.transition = animate && motion !== 'reduced' && motion !== 'off' ? EASE : 'none';
    el.style.transform = s === 1 ? '' : `scale(${s.toFixed(4)})`;
  };
  return {
    setInputs({ zoomable: z, want: w }) {
      if (z === zoomable && w === want) return;
      const animate = z && zoomable && w !== want;
      if (z !== zoomable) preview = null;
      zoomable = z;
      want = w;
      paint(animate);
    },
    setFill(f) {
      const next = Number.isFinite(f) && f >= 1 ? f : 1;
      if (next === fill) return;
      fill = next;
      paint(false);
    },
    preview(scale) {
      preview = scale;
      paint(false);
    },
    endPreview() {
      if (preview === null) return;
      preview = null;
      paint(true);
    },
    isOn: () => zoomable && want,
    fillScale: () => fill,
    dispose() {
      disposed = true;
      el.style.transform = '';
      el.style.transition = '';
    },
  };
}

// The one phone player's surface (the single-player assumption liveVideo.ts
// documents), for the watch layer's pinch.
let registered: ZoomSurface | null = null;

/** Registers `s`; the returned function unregisters it only while it is still
 *  the registered one, so a late dispose of an old player never clears a new. */
export function registerZoomSurface(s: ZoomSurface): () => void {
  registered = s;
  return () => {
    if (registered === s) registered = null;
  };
}

export function currentZoomSurface(): ZoomSurface | null {
  return registered;
}
