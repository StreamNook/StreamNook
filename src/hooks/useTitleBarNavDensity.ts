import { useLayoutEffect, useRef, useState } from 'react';

/** 0: full tabs. 1: tighter tabs, no count. 2: icon tabs. */
export type NavDensity = 0 | 1 | 2;

// Slack on the way back up, so a width that lands exactly on a boundary does
// not flip the strip between two densities on every resize tick.
const STEP_UP_SLACK_PX = 8;

/**
 * How dense Home's tab strip has to be to fit in the title bar's centre,
 * between the two icon clusters, in one row.
 *
 * Each density's width is only known once it has been drawn, so the strip
 * steps down while it overflows and steps back up when the width it last had
 * one step up fits again. Both happen in a layout effect, so the correction
 * lands before paint. A remembered width that has gone stale (a count grew)
 * at worst costs one extra step up and straight back down, also before paint.
 */
export function useTitleBarNavDensity(
  navSlot: HTMLElement | null,
  strip: HTMLElement | null,
): NavDensity {
  const [density, setDensity] = useState<NavDensity>(0);
  const widthsRef = useRef<Partial<Record<NavDensity, number>>>({});

  useLayoutEffect(() => {
    if (!navSlot) return;
    const bar = navSlot.closest<HTMLElement>('[data-titlebar]');
    const left = bar?.querySelector<HTMLElement>('[data-titlebar-left]');
    const right = bar?.querySelector<HTMLElement>('[data-titlebar-right]');
    if (!bar || !left || !right || !strip) return;

    const measure = () => {
      const cs = getComputedStyle(bar);
      const inner = bar.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight);
      const gap = parseFloat(cs.columnGap) || 0;
      const room = inner - left.offsetWidth - right.offsetWidth - 2 * gap;
      const width = strip.offsetWidth;
      widthsRef.current[density] = width;
      if (width > room && density < 2) {
        setDensity((density + 1) as NavDensity);
        return;
      }
      if (density > 0) {
        const up = (density - 1) as NavDensity;
        const upWidth = widthsRef.current[up];
        if (upWidth !== undefined && upWidth + STEP_UP_SLACK_PX <= room) setDensity(up);
      }
    };

    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(bar);
    ro.observe(left);
    ro.observe(right);
    ro.observe(strip);
    return () => ro.disconnect();
    // An element rather than a ref: a category drill-down unmounts the strip,
    // and the observer has to follow it when it comes back.
  }, [navSlot, strip, density]);

  return density;
}
