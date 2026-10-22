import { dragDivisor, dragFrac, dragSpeedLabel, NUDGE_SETTLE_MS, wheelStepSecs } from './scrubPrecision';

/**
 * Fine control on Plyr's VOD seek bar: the two gestures the live broadcast
 * timeline has (see scrubPrecision). A drag slows to a quarter and then a
 * sixteenth as the pointer moves away from the bar, and a wheel notch over
 * the bar moves ten seconds (Shift a minute, Ctrl a second). On a long VOD a
 * pixel of the bar is tens of seconds, so without them a specific moment is
 * out of reach.
 *
 * The bar is Plyr's native range, and Chromium drags a native range itself,
 * so the press is taken from it: `preventDefault` on pointerdown stops the
 * range's own drag and the mouse events that would follow it. The drag is
 * integrated here, and every move seeks through Plyr's own input handler
 * with the exact fraction (`seek-value`, which Plyr prefers over the range's
 * stepped value). A synthetic mousemove at the thumb then feeds everything
 * that follows the pointer (Plyr's tooltip, its storyboard preview, the
 * chapter tip, the overlay's idle timer), so all of them name the moment a
 * release will land on, even once the thumb no longer sits under the
 * pointer. Plyr pauses while its range is held and resumes after; with its
 * mouse events gone that pair is done here.
 *
 * A wheel notch seeks at once: a VOD seek is cheap and shows the frame. (The
 * live timeline arms a notch and lands it later because a landing there can
 * be a relay swap.) Touch keeps the native range.
 *
 * Returns the cleanup.
 */
export function attachFineScrub(progress: HTMLElement, video: HTMLVideoElement): () => void {
  const input = progress.querySelector<HTMLInputElement>('input[data-plyr="seek"]');
  if (!input) return () => {};

  let drag: { frac: number; lastX: number; rect: DOMRect; resume: boolean } | null = null;
  // The real pointer while it is over the bar, so the tips can go back under
  // it once a wheel nudge settles.
  let pointer: { x: number; y: number } | null = null;
  // Where the last wheel notch landed, until the wheel goes quiet.
  let nudge: { frac: number; rect: DOMRect } | null = null;
  let nudgeTimer: number | null = null;

  const duration = () => (Number.isFinite(video.duration) && video.duration > 0 ? video.duration : 0);
  const clamp01 = (f: number) => Math.min(1, Math.max(0, f));
  // The progress box is exactly where the thumb's centre travels (see the
  // range geometry in globals.css), so a fraction of it is a thumb position.
  const thumbX = (rect: DOMRect, frac: number) => rect.left + frac * rect.width;
  const midY = (rect: DOMRect) => rect.top + rect.height / 2;

  /** Seeks to `frac` through Plyr's input handler: it moves the range's fill
   *  and seeks to `seek-value`, unrounded, rather than the stepped value. */
  const seekTo = (frac: number) => {
    const pct = String(frac * 100);
    input.value = pct;
    input.setAttribute('seek-value', pct);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  };

  /** Points every pointer-following tip at `x`, exactly. */
  const pointTipsAt = (x: number, y: number) => {
    const e = new MouseEvent('mousemove', {
      bubbles: true,
      cancelable: true,
      clientX: Math.round(x),
      clientY: Math.round(y),
      view: window,
    });
    // The constructor rounds to whole pixels; readers should see the point.
    Object.defineProperties(e, {
      clientX: { value: x },
      clientY: { value: y },
      pageX: { value: x + window.scrollX },
      pageY: { value: y + window.scrollY },
    });
    input.dispatchEvent(e);
  };

  /** "1/4 speed" in whichever tip is showing, through a custom property the
   *  tips' `::after` reads (globals.css); unset at full speed. */
  const showSpeed = (divisor: number) => {
    const label = dragSpeedLabel(divisor);
    if (label) progress.style.setProperty('--sn-scrub-speed', JSON.stringify(label));
    else progress.style.removeProperty('--sn-scrub-speed');
  };

  const clearNudge = () => {
    if (nudgeTimer != null) window.clearTimeout(nudgeTimer);
    nudgeTimer = null;
    nudge = null;
  };

  // Plyr hides its storyboard preview on every `seeked`, and both gestures
  // seek on every step, so the preview blinked off after each one and was
  // gone whenever the drag held still. Plyr's own handler runs first; putting
  // the tips back in the same task means the hide never reaches the screen.
  const onSeeked = () => {
    if (drag) pointTipsAt(thumbX(drag.rect, drag.frac), midY(drag.rect));
    else if (nudge) pointTipsAt(thumbX(nudge.rect, nudge.frac), midY(nudge.rect));
  };

  const onPointerDown = (e: PointerEvent) => {
    if (e.button !== 0 || !e.isPrimary || e.pointerType === 'touch' || e.target !== input) return;
    if (!duration()) return;
    const rect = progress.getBoundingClientRect();
    if (rect.width <= 0) return;
    e.preventDefault();
    input.setPointerCapture(e.pointerId);
    clearNudge();
    const frac = clamp01((e.clientX - rect.left) / rect.width);
    const resume = !video.paused;
    if (resume) video.pause();
    drag = { frac, lastX: e.clientX, rect, resume };
    progress.classList.add('sn-scrubbing');
    seekTo(frac);
    pointTipsAt(thumbX(rect, frac), midY(rect));
  };

  const onPointerMove = (e: PointerEvent) => {
    const g = drag;
    if (!g) {
      if (e.isTrusted) pointer = { x: e.clientX, y: e.clientY };
      return;
    }
    const divisor = dragDivisor(Math.abs(e.clientY - midY(g.rect)));
    const next = dragFrac(g.frac, e.clientX - g.lastX, g.rect.width, divisor);
    g.lastX = e.clientX;
    pointer = { x: e.clientX, y: e.clientY };
    showSpeed(divisor);
    if (next !== g.frac) {
      g.frac = next;
      seekTo(next);
    }
    // Every move, not just the ones that moved the thumb: the tips follow
    // the thumb and the overlay stays up while the drag is held.
    pointTipsAt(thumbX(g.rect, g.frac), midY(g.rect));
  };

  const endDrag = (resume: boolean) => {
    const g = drag;
    if (!g) return;
    drag = null;
    progress.classList.remove('sn-scrubbing');
    showSpeed(1);
    if (resume && g.resume) void video.play().catch(() => {});
    // Back to hover: the tips follow the pointer again if it is over the
    // bar (a slowed drag can end well away from the thumb).
    const r = progress.getBoundingClientRect();
    if (pointer && pointer.x >= r.left && pointer.x <= r.right && Math.abs(pointer.y - midY(r)) <= r.height / 2) {
      pointTipsAt(pointer.x, pointer.y);
    }
  };
  const onPointerUp = () => endDrag(true);
  const onLostCapture = () => endDrag(true);
  const onPointerLeave = () => {
    if (!drag) pointer = null;
  };

  const onWheel = (e: WheelEvent) => {
    const step = wheelStepSecs(e);
    const d = duration();
    if (!step || !d) return;
    e.preventDefault();
    e.stopPropagation();
    const g = drag;
    if (g) {
      // Mid-drag, the wheel fine-tunes the drag itself.
      g.frac = clamp01(g.frac + step / d);
      seekTo(g.frac);
      pointTipsAt(thumbX(g.rect, g.frac), midY(g.rect));
      return;
    }
    // From the last notch's landing: currentTime reports a seek in flight.
    const rect = progress.getBoundingClientRect();
    const frac = clamp01((video.currentTime + step) / d);
    seekTo(frac);
    pointTipsAt(thumbX(rect, frac), midY(rect));
    clearNudge();
    nudge = { frac, rect };
    nudgeTimer = window.setTimeout(() => {
      nudgeTimer = null;
      nudge = null;
      if (pointer && !drag) pointTipsAt(pointer.x, pointer.y);
    }, NUDGE_SETTLE_MS);
  };

  // The player turns a wheel over itself into volume; not over this bar.
  progress.setAttribute('data-no-wheel-volume', '');
  progress.addEventListener('pointerdown', onPointerDown);
  progress.addEventListener('pointermove', onPointerMove);
  progress.addEventListener('pointerup', onPointerUp);
  progress.addEventListener('pointercancel', onPointerUp);
  progress.addEventListener('lostpointercapture', onLostCapture);
  progress.addEventListener('pointerleave', onPointerLeave);
  progress.addEventListener('wheel', onWheel, { passive: false });
  video.addEventListener('seeked', onSeeked);
  return () => {
    video.removeEventListener('seeked', onSeeked);
    clearNudge();
    // A rebuild mid-drag: drop the drag without restarting a player that is
    // being torn down.
    endDrag(false);
    progress.removeAttribute('data-no-wheel-volume');
    progress.removeEventListener('pointerdown', onPointerDown);
    progress.removeEventListener('pointermove', onPointerMove);
    progress.removeEventListener('pointerup', onPointerUp);
    progress.removeEventListener('pointercancel', onPointerUp);
    progress.removeEventListener('lostpointercapture', onLostCapture);
    progress.removeEventListener('pointerleave', onPointerLeave);
    progress.removeEventListener('wheel', onWheel);
  };
}
