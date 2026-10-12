import { memo, useEffect, useRef, type RefObject } from 'react';
import { layoutTicks, type MarkedRow, type PlacedMark, type Tick } from './scrollbarMarkLayout';

// The tick track drawn over a chat's scrollbar. Rows are marked by Rust
// (`metadata.scrollbar_mark`); this places them where the rows sit in the
// browser's layout and paints one small bar per tick.
//
// Cost model, since chat is the hottest path in the app:
// - No React state. The ticks are a pool of plain elements written directly,
//   so a new message never re-renders anything here.
// - Layout is read, never measured per frame: one pass over the list's rows,
//   reading offsetTop/offsetHeight of the marked ones only, at most once per
//   MIN_INTERVAL_MS and inside an animation frame (where the frame's layout is
//   due anyway). Triggers are the list's own size changes (a ResizeObserver,
//   which fires when rows arrive, trim or settle) and a change of marks.
// - Scrolling costs nothing: a tick's place is the row's place in the content,
//   mapped onto the track with the same ratio the scrollbar thumb uses.
// - Ticks never move by animation. Over a macOS overlay scrollbar the layer
//   fades with the bar (opacity only), and reduced motion drops the fade.
// - Phones draw no scrollbar, so the list does not mount this there.

/** Fastest redraw rate while chat streams. Ticks drift by a pixel at most
 *  between draws; the list itself repaints far more often. */
const MIN_INTERVAL_MS = 120;

function tickKey(ticks: readonly Tick[]): string {
  let key = '';
  for (const t of ticks) key += `${t.y},${t.h},${t.kind},${t.color ?? ''}|`;
  return key;
}

function paint(layer: HTMLElement, ticks: readonly Tick[]) {
  const kids = layer.children;
  for (let i = 0; i < ticks.length; i++) {
    const t = ticks[i];
    let el = kids[i] as HTMLElement | undefined;
    if (!el) {
      el = document.createElement('i');
      layer.appendChild(el);
    }
    el.dataset.kind = t.kind;
    el.style.transform = `translateY(${t.y}px)`;
    el.style.height = `${t.h}px`;
    el.style.backgroundColor = t.color ?? '';
  }
  while (layer.children.length > ticks.length) layer.lastElementChild!.remove();
}

/** Where each marked row sits in the scroll content. Rows share one offset
 *  parent with the content wrapper, so the difference is the row's place in
 *  the content whatever is positioned above it. */
function placeRows(content: HTMLElement, marks: ReadonlyMap<string, MarkedRow>): PlacedMark[] {
  const base = content.offsetTop;
  const rows: PlacedMark[] = [];
  for (const child of content.children) {
    const id = child.getAttribute('data-message-id');
    if (!id) continue;
    const mark = marks.get(id);
    if (!mark) continue;
    const el = child as HTMLElement;
    rows.push({ ...mark, top: el.offsetTop - base, height: el.offsetHeight });
  }
  return rows;
}

export interface ScrollbarMarksProps {
  /** The scrolling element whose scrollbar the ticks sit on. */
  scrollerRef: RefObject<HTMLElement | null>;
  /** Its content wrapper, whose direct children are the rows (each carrying
   *  `data-message-id`), starting at the top of the scroll content. */
  contentRef: RefObject<HTMLElement | null>;
  /** Marked rows by id. A new map means the marks changed. */
  marks: ReadonlyMap<string, MarkedRow>;
  /** The platform's scrollbar overlays the content and shows only while in
   *  use (macOS). The ticks then show only while that bar would: while the
   *  viewer scrolls and a moment after, or while the pointer is at the edge. */
  overlay?: boolean;
}

/** How long overlay ticks stay after the last scroll input. */
const OVERLAY_LINGER_MS = 1000;
/** How close to the scroller's right edge the pointer brings overlay ticks up. */
const OVERLAY_EDGE_PX = 16;

export const ScrollbarMarks = memo(function ScrollbarMarks({
  scrollerRef,
  contentRef,
  marks,
  overlay = false,
}: ScrollbarMarksProps) {
  const layerRef = useRef<HTMLDivElement>(null);
  const marksRef = useRef(marks);
  const scheduleRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    const scroller = scrollerRef.current;
    const content = contentRef.current;
    const layer = layerRef.current;
    if (!scroller || !content || !layer) return;
    let raf = 0;
    let timer = 0;
    let last = -Infinity;
    let lastKey = '';

    const draw = () => {
      raf = 0;
      last = performance.now();
      const current = marksRef.current;
      const track = scroller.clientHeight;
      const total = scroller.scrollHeight;
      // Nothing to scroll means nothing is out of sight: no ticks.
      const ticks =
        current.size > 0 && total > track + 1 ? layoutTicks(placeRows(content, current), total, track) : [];
      const key = tickKey(ticks);
      if (key === lastKey) return;
      lastKey = key;
      paint(layer, ticks);
    };

    const schedule = () => {
      if (raf || timer) return;
      const wait = MIN_INTERVAL_MS - (performance.now() - last);
      if (wait > 0) {
        timer = window.setTimeout(() => {
          timer = 0;
          raf = requestAnimationFrame(draw);
        }, wait);
      } else {
        raf = requestAnimationFrame(draw);
      }
    };

    scheduleRef.current = schedule;
    const ro = new ResizeObserver(schedule);
    ro.observe(content);
    ro.observe(scroller);
    schedule();
    return () => {
      scheduleRef.current = null;
      ro.disconnect();
      if (raf) cancelAnimationFrame(raf);
      if (timer) clearTimeout(timer);
    };
  }, [scrollerRef, contentRef]);

  useEffect(() => {
    marksRef.current = marks;
    scheduleRef.current?.();
  }, [marks]);

  // Overlay scrollbars show on scroll input (wheel, trackpad, touch), not on
  // the list following new rows, and while the pointer is over the bar.
  useEffect(() => {
    const scroller = scrollerRef.current;
    const layer = layerRef.current;
    if (!overlay || !scroller || !layer) return;
    let lingering = 0;
    let atEdge = false;
    const sync = () => {
      if (atEdge || lingering) layer.dataset.show = '';
      else delete layer.dataset.show;
    };
    const onScrollInput = () => {
      if (lingering) clearTimeout(lingering);
      lingering = window.setTimeout(() => {
        lingering = 0;
        sync();
      }, OVERLAY_LINGER_MS);
      sync();
    };
    const onPointerMove = (e: PointerEvent) => {
      const right = scroller.getBoundingClientRect().right;
      const near = e.clientX >= right - OVERLAY_EDGE_PX && e.clientX <= right;
      if (near !== atEdge) {
        atEdge = near;
        sync();
      }
    };
    const onPointerLeave = () => {
      if (!atEdge) return;
      atEdge = false;
      sync();
    };
    scroller.addEventListener('wheel', onScrollInput, { passive: true });
    scroller.addEventListener('touchmove', onScrollInput, { passive: true });
    scroller.addEventListener('pointermove', onPointerMove, { passive: true });
    scroller.addEventListener('pointerleave', onPointerLeave);
    return () => {
      scroller.removeEventListener('wheel', onScrollInput);
      scroller.removeEventListener('touchmove', onScrollInput);
      scroller.removeEventListener('pointermove', onPointerMove);
      scroller.removeEventListener('pointerleave', onPointerLeave);
      if (lingering) clearTimeout(lingering);
      delete layer.dataset.show;
    };
  }, [scrollerRef, overlay]);

  return (
    <div ref={layerRef} className="sn-scroll-marks" data-overlay={overlay ? '' : undefined} aria-hidden="true" />
  );
});

export default ScrollbarMarks;
