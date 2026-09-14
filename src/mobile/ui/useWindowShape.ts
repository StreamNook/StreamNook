// The shape of the window we are actually running in.
//
// Two separate facts, and conflating them is what makes apps look wrong on
// foldables:
//
//  1. HOW MUCH ROOM there is (size class). Android's own breakpoints, because
//     they are the ones device makers design to.
//  2. WHETHER A HINGE CROSSES THE SCREEN, and where. A Z Fold unfolds with the
//     crease running VERTICALLY down the middle, so on that device the centre of
//     the viewport is the single worst place to put a control. Aligning a
//     two-pane split TO the seam is the whole trick; centring anything on it is
//     the bug.
//
// Orientation alone cannot express either. An unfolded Fold is roughly 840x757,
// an aspect ratio near 1.1, so "portrait vs landscape" is close to meaningless
// there — which is exactly why this replaces those decisions.
import { useEffect, useState } from 'react';

/** Android window size classes, in CSS px (which track dp here). */
export type SizeClass = 'compact' | 'medium' | 'expanded';

export interface FoldInfo {
  /** True when the hinge runs top-to-bottom, splitting the screen left/right. */
  vertical: boolean;
  /** 'half' is the tent/book half-open posture; 'flat' is fully opened. */
  posture: 'flat' | 'half';
  /** Hinge rect in CSS px. */
  x: number;
  width: number;
  y: number;
  height: number;
}

export interface WindowShape {
  w: number;
  h: number;
  sizeClass: SizeClass;
  landscape: boolean;
  /** Null when there is no hinge (ordinary phone, tablet, folded Fold). */
  fold: FoldInfo | null;
  /**
   * Where a two-pane split should fall, in CSS px from the left edge.
   * Snaps to the hinge when one crosses vertically, so neither pane is bisected;
   * otherwise a plain proportional split.
   */
  splitX: number;
  /** Whether there is room for two panes side by side at all. */
  twoPane: boolean;
}

function classify(w: number): SizeClass {
  if (w < 600) return 'compact';
  if (w < 840) return 'medium';
  return 'expanded';
}

export function useWindowShape(): WindowShape {
  const [size, setSize] = useState(() => ({
    w: window.innerWidth,
    h: window.innerHeight,
  }));
  const [fold, setFold] = useState<FoldInfo | null>(null);

  useEffect(() => {
    const onResize = () => setSize({ w: window.innerWidth, h: window.innerHeight });
    window.addEventListener('resize', onResize);
    // Folding does not always fire resize before the layout settles, so listen
    // to the orientation change too.
    window.addEventListener('orientationchange', onResize);
    return () => {
      window.removeEventListener('resize', onResize);
      window.removeEventListener('orientationchange', onResize);
    };
  }, []);

  useEffect(() => {
    const onFold = (e: Event) => setFold((e as CustomEvent<FoldInfo | null>).detail ?? null);
    window.addEventListener('sn:fold', onFold);
    return () => window.removeEventListener('sn:fold', onFold);
  }, []);

  const sizeClass = classify(size.w);
  const landscape = size.w > size.h;

  // Two panes need real width. `medium` qualifies only in landscape, where the
  // height would otherwise leave each stacked pane too short to use.
  const twoPane = sizeClass === 'expanded' || (sizeClass === 'medium' && landscape);

  // A vertical hinge dictates the split. Otherwise favour the player: chat needs
  // less width than video does.
  const splitX =
    fold?.vertical && fold.x > 0 && fold.x < size.w
      ? fold.x + fold.width / 2
      : Math.round(size.w * 0.62);

  return { w: size.w, h: size.h, sizeClass, landscape, fold, splitX, twoPane };
}
