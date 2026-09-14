// Long-press to arm, then keep dragging in the same gesture.
//
// This cannot be built on the `contextmenu` event that the existing action sheet
// uses. Android does synthesise contextmenu from a hold, but it fires only after
// the hold completes and carries no continuation, so there is no way to keep
// tracking the same finger into a drag. Raw pointer events are the only route.
//
// The hard part is coexisting with chat scrolling. A vertical drag has to stay a
// scroll, so movement before the hold fires CANCELS the press. Once armed, the
// list is locked (touch-action: none) for the rest of the gesture so the browser
// stops trying to scroll underneath the fan.
//
// Stopping the scroll needs `preventDefault()` on a NON-PASSIVE touchmove, not
// `touch-action: none`. Chromium decides whether a gesture scrolls at touchstart
// and does not re-read touch-action mid-gesture, so setting it on arm did
// nothing: the list kept panning, Android fired pointercancel on the original
// pointer, and the fan tore itself down. Same lesson the pull-to-refresh in this
// tree already learned.
import { useCallback, useEffect, useRef } from 'react';
import { hapticArm } from '../ui/haptics';

const HOLD_MS = 420;
/** Movement beyond this before the hold fires means the user is scrolling. */
const CANCEL_SLOP_PX = 10;

interface Options {
  /** Resolve the pressed element to a message id, or null to ignore the press. */
  resolve: (el: HTMLElement) => string | null;
  onArm: (messageId: string, x: number, y: number) => void;
  /** Element to lock scrolling on while armed. */
  scrollLockRef: React.RefObject<HTMLElement | null>;
}

export function useLongPressDrag({ resolve, onArm, scrollLockRef }: Options) {
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const start = useRef<{ x: number; y: number; id: string } | null>(null);
  const armed = useRef(false);

  const clearTimer = useCallback(() => {
    if (timer.current) {
      clearTimeout(timer.current);
      timer.current = null;
    }
  }, []);

  // No touch-action juggling: see the note at the top of this file. The
  // non-passive touchmove listener below is what actually holds the scroll.
  const unlock = useCallback(() => {}, []);

  /** Call when the fan closes so a new press can arm again. */
  const release = useCallback(() => {
    armed.current = false;
    start.current = null;
    clearTimer();
    unlock();
  }, [clearTimer, unlock]);

  const onPointerDown = useCallback(
    (e: React.PointerEvent) => {
      // Only primary touch/pen. A stray second finger must not re-arm.
      if (!e.isPrimary) return;
      const row = (e.target as HTMLElement).closest('[data-message-id]') as HTMLElement | null;
      if (!row) return;
      const id = resolve(row);
      if (!id) return;
      start.current = { x: e.clientX, y: e.clientY, id };
      armed.current = false;
      clearTimer();
      timer.current = setTimeout(() => {
        const s = start.current;
        if (!s) return;
        armed.current = true;
        hapticArm();
        onArm(s.id, s.x, s.y);
      }, HOLD_MS);
    },
    [resolve, onArm, clearTimer],
  );

  const onPointerMove = useCallback(
    (e: React.PointerEvent) => {
      const s = start.current;
      if (!s || armed.current) return;
      if (Math.hypot(e.clientX - s.x, e.clientY - s.y) > CANCEL_SLOP_PX) {
        // Became a scroll. Abandon the press entirely.
        clearTimer();
        start.current = null;
      }
    },
    [clearTimer],
  );

  const onPointerUp = useCallback(() => {
    // If the hold never fired this was a tap or a scroll; the fan owns pointerup
    // once armed, so nothing to do here in that case.
    if (!armed.current) {
      clearTimer();
      start.current = null;
    }
  }, [clearTimer]);

  // A pointercancel means the browser took the gesture over. Abandon the pending
  // hold rather than arming a fan the user is no longer driving.
  const onPointerCancel = useCallback(() => {
    if (!armed.current) {
      clearTimer();
      start.current = null;
    }
  }, [clearTimer]);

  // The one thing that keeps the list still while the fan is up. Registered
  // natively because React's synthetic touchmove is passive and cannot
  // preventDefault, and attached for the component's lifetime rather than on arm
  // so the listener is already in place when the gesture begins.
  //
  // KNOWN COST, deliberately left in place: a lifetime non-passive touchmove on
  // an ancestor of the scroller takes this subtree off Chromium's compositor
  // scrolling fast path, so every touchmove round-trips to the main thread. That
  // contributed to chat swipes dropping their first few pixels. The dominant
  // cause of that symptom was the pause/resume re-render storm in
  // MobileChatPane, now fixed there, which leaves this a secondary cost.
  //
  // Moving the attach to arm-time LOOKS free and is not: Chromium fixes a
  // gesture's disposition at touchstart and does not re-read handler regions
  // mid-sequence, so a listener added 420ms into a hold can simply be ignored
  // for the in-flight touch. Do not change this without device-verifying that
  // the fan still freezes the list.
  useEffect(() => {
    const el = scrollLockRef.current;
    if (!el) return;
    const onTouchMove = (e: TouchEvent) => {
      if (armed.current) e.preventDefault();
    };
    el.addEventListener('touchmove', onTouchMove, { passive: false });
    return () => el.removeEventListener('touchmove', onTouchMove);
  }, [scrollLockRef]);

  useEffect(() => release, [release]);

  return {
    onPointerDown,
    onPointerMove,
    onPointerUp,
    onPointerCancel,
    release,
    isArmed: () => armed.current,
  };
}
