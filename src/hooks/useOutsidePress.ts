import { useCallback, useEffect, useEffectEvent, useRef } from 'react';
import type { RefObject } from 'react';

/** What counts as the menu: a ref, or a selector for a part drawn by another
 *  component that the menu holds no ref to (a toggle in a header). */
export type OutsidePressInside = RefObject<Element | null> | string;

/**
 * Closes a menu on a press anywhere outside it. Every chat menu lets go this
 * one way, so a click anywhere does the same thing whichever menu is open.
 *
 * - `inside` is everything that belongs to the menu: its surface and the
 *   button that opens it. That button toggles on its own click, so a press on
 *   it never counts as outside (otherwise the press closes the menu and the
 *   click opens it straight back).
 * - Heard in the capture phase, so a surface that swallows its own presses
 *   (the player, a tile toolbar) still counts as outside, and nothing needs a
 *   backdrop over the page to catch the click.
 * - Put the returned handler on the menu's root as `onPointerDownCapture`.
 *   React carries a press through portals, so whatever the menu portals out
 *   (a dialog, an expanded view) reports itself as inside although it sits
 *   elsewhere in the page. That report arrives after the capture listener, so
 *   the verdict waits until the press has finished dispatching.
 */
export function useOutsidePress(
  open: boolean,
  inside: ReadonlyArray<OutsidePressInside | null | undefined>,
  onOutside: () => void,
): () => void {
  const pressedInsideRef = useRef(false);
  const isInside = useEffectEvent((target: Element) =>
    inside.some((part) =>
      typeof part === 'string' ? target.closest(part) !== null : !!part?.current?.contains(target),
    ),
  );
  const close = useEffectEvent(() => onOutside());

  useEffect(() => {
    if (!open) return;
    let verdict: ReturnType<typeof setTimeout> | undefined;
    const onDown = (e: PointerEvent) => {
      pressedInsideRef.current = false;
      if (!(e.target instanceof Element) || isInside(e.target)) return;
      clearTimeout(verdict);
      verdict = setTimeout(() => {
        if (!pressedInsideRef.current) close();
      }, 0);
    };
    document.addEventListener('pointerdown', onDown, true);
    return () => {
      document.removeEventListener('pointerdown', onDown, true);
      clearTimeout(verdict);
    };
  }, [open]);

  return useCallback(() => {
    pressedInsideRef.current = true;
  }, []);
}
