// Keeps a menu that grows upward from the message box inside the room it has.
//
// The chat box's menus (the insert menu, Tools) open above the box at a fixed
// tall height. In a short window, or with a banner such as the watch streak
// pushing the box up, that height ran past the top of the chat column, whose
// `overflow: hidden` cut the menu's top off: the search field and anything
// pinned to the top went missing. This caps the menu's max-height at the space
// between its bottom edge and the top of the nearest clipping ancestor, and
// follows window and column resizes while the menu is open.
import { useLayoutEffect, type RefObject } from 'react';

/** Room kept clear above the menu, so it never touches the column's top edge. */
const TOP_GAP_PX = 8;
/** Below this the menu stops shrinking; a column that short clips regardless. */
const MIN_PX = 160;

/** The max-height for a menu whose bottom edge sits at `menuBottom`, inside a
 *  clipping box whose top edge is at `clipTop` (both viewport px). */
export function roomAbove(menuBottom: number, clipTop: number, maxPx: number): number {
  const room = Math.floor(menuBottom - clipTop - TOP_GAP_PX);
  return Math.max(MIN_PX, Math.min(maxPx, room));
}

function clippingAncestor(el: HTMLElement): HTMLElement | null {
  for (let p = el.parentElement; p && p !== document.body; p = p.parentElement) {
    const { overflowY } = getComputedStyle(p);
    if (overflowY !== 'visible') return p;
  }
  return null;
}

/** Caps `ref`'s max-height to the room above it while `active`. Writes the
 *  style straight to the element: a resize should not re-render the menu. */
export function useFitAbove(ref: RefObject<HTMLElement | null>, active: boolean, maxPx: number): void {
  useLayoutEffect(() => {
    const el = ref.current;
    if (!active || !el) return;
    const clip = clippingAncestor(el);
    const fit = () => {
      const clipTop = clip ? clip.getBoundingClientRect().top : 0;
      el.style.maxHeight = `${roomAbove(el.getBoundingClientRect().bottom, clipTop, maxPx)}px`;
    };
    fit();
    const observer = new ResizeObserver(fit);
    if (clip) observer.observe(clip);
    window.addEventListener('resize', fit);
    return () => {
      observer.disconnect();
      window.removeEventListener('resize', fit);
    };
  }, [ref, active, maxPx]);
}
