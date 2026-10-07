import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';

/** How far the row fades at an edge with tabs scrolled behind it. */
export const TAB_FADE_PX = 24;

/**
 * A row of tabs that scrolls sideways with no scrollbar: a bar under a row of
 * pills is chrome, not information. Overflow shows as a fade on each edge that
 * has tabs behind it, a vertical mouse wheel scrolls the row, and the active
 * tab (the child carrying `data-tab-key={activeKey}`) is kept in view, clear
 * of the fade.
 *
 * It takes only the width its tabs need, so buttons placed after it follow
 * the last tab while the tabs fit.
 */
export function TabScroller({
  activeKey,
  count,
  className = '',
  onOverflowChange,
  unfolded = false,
  children,
}: {
  activeKey: string | null;
  /** The number of tabs, so a tab added past the edge is scrolled to. */
  count: number;
  className?: string;
  /** Told whether the tabs are wider than the row, whenever that changes. */
  onOverflowChange?: (overflowing: boolean) => void;
  /** Stop scrolling and let the tabs wrap (the caller wraps its tab row and
   *  places this element). Overflow is not re-measured meanwhile: unfolded,
   *  the tabs always fit, which would read as "no longer overflowing". */
  unfolded?: boolean;
  children: ReactNode;
}) {
  const scrollerRef = useRef<HTMLDivElement>(null);
  const unfoldedRef = useRef(unfolded);
  useEffect(() => {
    unfoldedRef.current = unfolded;
  });
  const [edges, setEdges] = useState({ left: false, right: false });
  const overflowCb = useRef(onOverflowChange);
  useEffect(() => {
    overflowCb.current = onOverflowChange;
  });
  const overflowing = useRef<boolean | null>(null);
  const measureEdges = useCallback(() => {
    const el = scrollerRef.current;
    if (!el || unfoldedRef.current) return;
    const left = el.scrollLeft > 1;
    const right = el.scrollLeft + el.clientWidth < el.scrollWidth - 1;
    setEdges((prev) => (prev.left === left && prev.right === right ? prev : { left, right }));
    const over = el.scrollWidth > el.clientWidth + 1;
    if (overflowing.current !== over) {
      overflowing.current = over;
      overflowCb.current?.(over);
    }
  }, []);
  useEffect(() => {
    const el = scrollerRef.current;
    if (!el) return;
    // Re-measure when the row resizes or the tabs change width (added,
    // removed, renamed). A vertical mouse wheel turns into a sideways scroll;
    // trackpads already send deltaX, so only a mostly-vertical wheel is taken.
    const observer = new ResizeObserver(measureEdges);
    observer.observe(el);
    if (el.firstElementChild) observer.observe(el.firstElementChild);
    const onWheel = (e: WheelEvent) => {
      if (Math.abs(e.deltaY) <= Math.abs(e.deltaX) || el.scrollWidth <= el.clientWidth) return;
      e.preventDefault();
      el.scrollLeft += e.deltaMode === 1 ? e.deltaY * 16 : e.deltaY;
    };
    el.addEventListener('wheel', onWheel, { passive: false });
    measureEdges();
    return () => {
      observer.disconnect();
      el.removeEventListener('wheel', onWheel);
    };
  }, [measureEdges]);
  // Scroll a tab fully clear of the edge fades. Only for the ACTIVE tab: a
  // row that scrolled toward the tab under the pointer moved it away from the
  // cursor.
  const bringIntoView = useCallback((tab: Element) => {
    const el = scrollerRef.current;
    if (!el) return;
    const t = tab.getBoundingClientRect();
    const s = el.getBoundingClientRect();
    // Only where a fade is actually drawn: an edge with nothing behind it has none.
    if (t.left < s.left + TAB_FADE_PX && el.scrollLeft > 1) {
      el.scrollBy({ left: t.left - s.left - TAB_FADE_PX, behavior: 'smooth' });
    } else if (t.right > s.right - TAB_FADE_PX && el.scrollLeft + el.clientWidth < el.scrollWidth - 1) {
      el.scrollBy({ left: t.right - s.right + TAB_FADE_PX, behavior: 'smooth' });
    }
  }, []);
  // A tab added past the right edge becomes the active one: bring it into view,
  // clear of the fade, instead of leaving it scrolled out of sight.
  useEffect(() => {
    const el = scrollerRef.current;
    if (!el || !activeKey) return;
    const tab = el.querySelector<HTMLElement>(`[data-tab-key="${CSS.escape(activeKey)}"]`);
    if (tab) bringIntoView(tab);
  }, [activeKey, count, bringIntoView]);
  const edgeMask =
    edges.left || edges.right
      ? `linear-gradient(to right, ${edges.left ? `transparent, #000 ${TAB_FADE_PX}px` : '#000'}, ${
          edges.right ? `#000 calc(100% - ${TAB_FADE_PX}px), transparent` : '#000'
        })`
      : undefined;

  // The standard scrollbar-width (not ::-webkit-scrollbar) hides the bar, so it
  // also beats the macOS thin-bar rule in globals.css.
  return (
    <div
      ref={scrollerRef}
      onScroll={measureEdges}
      data-tab-scroller
      className={`flex min-w-0 flex-initial items-center overflow-x-auto ${className}`}
      style={
        unfolded
          ? { overflow: 'visible' }
          : { scrollbarWidth: 'none', maskImage: edgeMask, WebkitMaskImage: edgeMask }
      }
    >
      {children}
    </div>
  );
}
