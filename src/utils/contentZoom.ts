import { useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import { isWindowHidden } from './windowVisibility';
import { Logger } from './logger';

/**
 * Zoom a stream to the picture it actually carries.
 *
 * TikTok sends a 9:16 frame whatever is in it. When a LIVE goes to two people
 * (co-host), the two feeds sit in a band across the middle and the rest of the
 * frame is black, baked into the video. Shown whole at full height, the people
 * end up in a small box in the middle of the player. This finds the black bars
 * INSIDE the frame and scales the video so the picture fills the player again;
 * the bars are clipped away by the player. A frame with no bars is left alone.
 *
 * It reads pixels, so it has to live where the decoded frames are: a 36x64
 * sample when Rust says the layout is changing, and a slow safety net between.
 */

/** Sample size. 9:16, like the streams this is for. */
const SW = 36;
const SH = 64;
/** A pixel this dark (0-255 luma) counts as bar. Encoded black is ~16. */
const BLACK_LUMA = 28;
/** Share of a row or column that must be dark for it to count as bar. */
const ROW_DARK_SHARE = 0.97;
/** Each bar must be at least this share of the frame to be worth zooming for. */
const MIN_BAR = 0.04;
/** A picture smaller than this is a dark scene, not a layout. */
const MIN_PICTURE = 0.25;
/** Never magnify beyond this, whatever the frame says. */
const MAX_ZOOM = 3;
/** The slow check between layout events, for a change nothing announced. */
const SAFETY_NET_MS = 5000;
/** Looks after a layout event: the message can land a moment before or after
 *  the video itself changes, and a change applies on two agreeing readings. */
const BURST_MS = [0, 250, 600, 1200, 2500];
/** Waiting for the first frame never hides a picture longer than this. There
 *  is nothing to show before a frame exists, so the wait itself is free; this
 *  only guards against a frame that is somehow never readable. */
const FIRST_FRAME_GIVE_UP_MS = 15000;

/** The picture's box inside the frame, as shares of it (0..1). */
export interface Box {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

export const FULL: Box = { x0: 0, y0: 0, x1: 1, y1: 1 };

/**
 * Where the picture is in an RGBA frame of `w` x `h`.
 *
 * A side counts as bar only when its OPPOSITE side is bar too (top with
 * bottom, left with right), which is what a layout's letterbox looks like and a
 * dark ceiling or a shadowed wall does not. Anything else answers FULL.
 */
export function contentBox(d: Uint8ClampedArray, w: number, h: number): Box {
  const dark = (i: number) => 0.2126 * d[i] + 0.7152 * d[i + 1] + 0.0722 * d[i + 2] < BLACK_LUMA;
  const rowDark = (y: number) => {
    let n = 0;
    for (let x = 0; x < w; x++) if (dark((y * w + x) * 4)) n++;
    return n >= w * ROW_DARK_SHARE;
  };
  const colDark = (x: number, y0: number, y1: number) => {
    let n = 0;
    for (let y = y0; y < y1; y++) if (dark((y * w + x) * 4)) n++;
    return n >= (y1 - y0) * ROW_DARK_SHARE;
  };

  let top = 0;
  while (top < h && rowDark(top)) top++;
  let bottom = h;
  while (bottom > top && rowDark(bottom - 1)) bottom--;
  // Both or neither.
  if (top / h < MIN_BAR || (h - bottom) / h < MIN_BAR) {
    top = 0;
    bottom = h;
  }
  if (bottom <= top) return FULL;

  let left = 0;
  while (left < w && colDark(left, top, bottom)) left++;
  let right = w;
  while (right > left && colDark(right - 1, top, bottom)) right--;
  if (left / w < MIN_BAR || (w - right) / w < MIN_BAR) {
    left = 0;
    right = w;
  }
  if (right <= left) return FULL;

  const box = { x0: left / w, y0: top / h, x1: right / w, y1: bottom / h };
  // A mostly black frame is a dark scene or a fade, not a layout to zoom into.
  if (box.x1 - box.x0 < MIN_PICTURE || box.y1 - box.y0 < MIN_PICTURE) return FULL;
  return box;
}

export function sameBox(a: Box, b: Box, tolerance = 0.03): boolean {
  return (
    Math.abs(a.x0 - b.x0) <= tolerance &&
    Math.abs(a.y0 - b.y0) <= tolerance &&
    Math.abs(a.x1 - b.x1) <= tolerance &&
    Math.abs(a.y1 - b.y1) <= tolerance
  );
}

/**
 * The CSS transform that fits `box` of a `vw` x `vh` video, drawn with
 * `object-fit: contain` in a `ew` x `eh` element, to that element. Origin is
 * the element's centre. Identity for the full frame.
 */
export function zoomFor(box: Box, ew: number, eh: number, vw: number, vh: number): string {
  if (sameBox(box, FULL, 0) || ew <= 0 || eh <= 0 || vw <= 0 || vh <= 0) return '';
  const fit = Math.min(ew / vw, eh / vh);
  const dw = vw * fit;
  const dh = vh * fit;
  const bw = (box.x1 - box.x0) * dw;
  const bh = (box.y1 - box.y0) * dh;
  const s = Math.min(ew / bw, eh / bh, MAX_ZOOM);
  if (s <= 1.02) return '';
  // Move the picture's centre to the element's centre, then scale about it.
  const tx = -s * ((box.x0 + box.x1) / 2 - 0.5) * dw;
  const ty = -s * ((box.y0 + box.y1) / 2 - 0.5) * dh;
  return `translate(${tx.toFixed(1)}px, ${ty.toFixed(1)}px) scale(${s.toFixed(3)})`;
}

/**
 * Keep `video` zoomed to its picture while `enabled`. The element's parent must
 * clip (the player does), since the zoomed bars fall outside it.
 *
 * When to look is Rust's call, not a clock's: the TikTok chat connection sees
 * a co-host join or leave and emits `provider-stream-layout` for `channel`,
 * and a burst of readings follows. Between events a slow safety net catches a
 * change nothing announced. The start is special: the picture is held back
 * until its first frame has been measured, so a co-host LIVE opens already
 * filling the player instead of visibly growing into it.
 */
export function useContentZoom(
  video: React.RefObject<HTMLVideoElement | null>,
  enabled: boolean,
  channel: string | null,
) {
  useEffect(() => {
    const el0 = video.current;
    if (!enabled || !el0) return;
    const el = el0;
    const canvas = document.createElement('canvas');
    canvas.width = SW;
    canvas.height = SH;
    const ctx = canvas.getContext('2d', { willReadFrequently: true });
    if (!ctx) return;

    let applied: Box = FULL;
    let candidate: Box | null = null;
    let stopped = false;
    let revealed = false;
    let disposed = false;
    const timers = new Set<ReturnType<typeof setTimeout>>();
    const after = (ms: number, fn: () => void) => {
      const t = setTimeout(() => {
        timers.delete(t);
        fn();
      }, ms);
      timers.add(t);
    };

    const paint = (animate: boolean) => {
      el.style.setProperty('transition', animate ? 'transform 350ms ease' : 'none');
      const t = zoomFor(applied, el.clientWidth, el.clientHeight, el.videoWidth, el.videoHeight);
      if (el.style.getPropertyValue('transform') !== t) el.style.setProperty('transform', t);
    };
    const reveal = () => {
      if (revealed) return;
      revealed = true;
      el.style.removeProperty('opacity');
    };

    const read = (): Box | null => {
      if (stopped || el.readyState < 2 || el.videoWidth === 0) return null;
      try {
        ctx.drawImage(el, 0, 0, SW, SH);
        return contentBox(ctx.getImageData(0, 0, SW, SH).data, SW, SH);
      } catch {
        // A protected stream cannot be read and never will be.
        stopped = true;
        reveal();
        return null;
      }
    };

    // A change applies on a second matching reading, so one odd frame (a cut to
    // black, a fade) never pumps the zoom.
    const sample = () => {
      if (isWindowHidden()) return;
      const seen = read();
      if (!seen) return;
      if (sameBox(seen, applied)) {
        candidate = null;
        return;
      }
      if (candidate && sameBox(seen, candidate)) {
        applied = seen;
        candidate = null;
        Logger.debug('[ContentZoom] picture box', applied);
        paint(true);
      } else {
        candidate = seen;
      }
    };
    const burst = () => BURST_MS.forEach((ms) => after(ms, sample));

    // The first frame decides the opening zoom, applied without animating while
    // the picture is still hidden. A burst follows, in case that frame was a
    // black one from before the picture arrived.
    el.style.setProperty('transform-origin', 'center center');
    el.style.setProperty('opacity', '0');
    const started = performance.now();
    const first = () => {
      if (stopped || disposed) return;
      const seen = read();
      if (seen) {
        applied = seen;
        paint(false);
        reveal();
        burst();
        return;
      }
      if (performance.now() - started > FIRST_FRAME_GIVE_UP_MS) {
        reveal();
        burst();
        return;
      }
      // Cheap until a frame exists: `read` checks readyState before drawing.
      after(50, first);
    };
    first();

    const net = setInterval(sample, SAFETY_NET_MS);
    // The transform is in pixels of the current layout, so it follows resizes,
    // at once rather than animated.
    const resize = new ResizeObserver(() => paint(false));
    resize.observe(el);

    let unlisten: (() => void) | undefined;
    if (channel) {
      const mine = channel.toLowerCase();
      void listen<{ provider: string; channel: string }>('provider-stream-layout', (e) => {
        if (e.payload.provider === 'tiktok' && e.payload.channel.toLowerCase() === mine) burst();
      }).then((u) => {
        if (disposed) u();
        else unlisten = u;
      });
    }

    return () => {
      stopped = true;
      disposed = true;
      clearInterval(net);
      timers.forEach(clearTimeout);
      timers.clear();
      resize.disconnect();
      unlisten?.();
      el.style.removeProperty('transform');
      el.style.removeProperty('transition');
      el.style.removeProperty('transform-origin');
      el.style.removeProperty('opacity');
    };
  }, [video, enabled, channel]);
}
