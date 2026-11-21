import type { CSSProperties } from 'react';

/** How far a list's edge fades where more of it lies beyond. */
export const FADE_PX = 14;

/** Which edges of a scroller have more content beyond them. */
export interface ScrollEdges {
  top: boolean;
  bottom: boolean;
}

/** Reads a scroller's edges: an edge counts when more than 2 px lies beyond it. */
export function readEdges(el: { scrollTop: number; scrollHeight: number; clientHeight: number }): ScrollEdges {
  return {
    top: el.scrollTop > 2,
    bottom: el.scrollHeight - el.scrollTop - el.clientHeight > 2,
  };
}

/**
 * The soft edge a scroller takes where more of its content lies beyond it, so a row or a
 * line cut by the card's edge reads as "more" and never as a hard slice. None where the
 * scroller sits at that end, so a card that fits keeps its own edge and corners.
 */
export function edgeFade(edges: ScrollEdges): CSSProperties | undefined {
  if (!edges.top && !edges.bottom) return undefined;
  const image = `linear-gradient(to bottom, transparent 0, #000 ${edges.top ? FADE_PX : 0}px, #000 calc(100% - ${
    edges.bottom ? FADE_PX : 0
  }px), transparent 100%)`;
  return { maskImage: image, WebkitMaskImage: image };
}
