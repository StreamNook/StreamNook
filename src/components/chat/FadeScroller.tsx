import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { edgeFade, readEdges, type ScrollEdges } from './edgeFade';

/**
 * The scrolling body of a stack card that Rust held to a height: it scrolls, and fades at the
 * edge that has more beyond it. The mask sits on this scrolling content and never on the
 * glass card around it (a mask on an ancestor would make that card the backdrop root of the
 * glass inside it and flatten its blur). `data-fade-scroll` lets the stack's card measure the
 * natural height of what is scrolled.
 */
export function FadeScroller({ className = '', children }: { className?: string; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  const [edges, setEdges] = useState<ScrollEdges>({ top: false, bottom: false });
  const sync = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    const next = readEdges(el);
    setEdges((prev) => (prev.top === next.top && prev.bottom === next.bottom ? prev : next));
  }, []);
  useEffect(() => {
    const el = ref.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
    sync();
    const ro = new ResizeObserver(sync);
    ro.observe(el);
    Array.from(el.children).forEach((c) => ro.observe(c));
    return () => ro.disconnect();
  }, [sync]);
  return (
    <div
      ref={ref}
      data-fade-scroll=""
      className={`min-h-0 overflow-y-auto overscroll-contain ${className}`}
      style={edgeFade(edges)}
      onScroll={sync}
    >
      {children}
    </div>
  );
}
