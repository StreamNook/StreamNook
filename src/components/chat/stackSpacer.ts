// The floating top-of-chat stack never shrinks the chat list (SN-284): the list
// keeps its whole column and gets a spacer at its top instead, so the oldest
// row can scroll out from under the stack. This file is the arithmetic the
// three desktop lists and both phones share.

/** Where the chat's top-of-column chrome ends, in the column's own pixels. */
export interface TopChrome {
  /** The floating header's bottom (0 on the phones, which have none). */
  header: number;
  /** The combined-chat bar's bottom, in flow under the header (0 without one). */
  bar: number;
  /** The stack's drawn bottom (0 when it draws nothing). */
  stack: number;
}

/** How far down the column the floating chrome reaches: the lowest of the
 *  header, the bar under it and the stack. */
export function chromeBottom(c: TopChrome): number {
  return Math.max(0, c.header, c.bar, c.stack);
}

/**
 * The spacer at the top of the list: the stack's bottom minus the list's top.
 * The list's top is the combined-chat bar's bottom when one sits above it in
 * flow, else the column's top. It includes the header, which floats over the
 * list; it is zero when something in flow already cleared the stack.
 */
export function stackSpacer(c: TopChrome): number {
  const listTop = c.bar > 0 ? c.bar : 0;
  return Math.max(0, Math.round(Math.max(c.header, c.stack) - listTop));
}

/**
 * How far to move `scrollTop` in the same layout pass the spacer changed in.
 * A browser with overflow anchoring keeps the rows in place by itself (Chromium);
 * one without (WebKit) would show them jump by the spacer's change.
 */
export function spacerScrollDelta(prev: number, next: number, anchorSupported: boolean): number {
  if (anchorSupported) return 0;
  return next - prev;
}

/** Whether this browser anchors scroll to the rows when content above grows. */
export function supportsOverflowAnchor(): boolean {
  try {
    return typeof CSS !== 'undefined' && typeof CSS.supports === 'function' && CSS.supports('overflow-anchor', 'auto');
  } catch {
    return false;
  }
}

/**
 * The part of a scroll container's top the floating stack covers, for a jump:
 * the stack's drawn bottom (the column's pixels) minus where the container
 * starts in the column. A jump centers the row in what is left below it.
 */
export function coveredTop(stackBottom: number, containerTop: number, columnTop: number): number {
  return Math.max(0, Math.round(columnTop + stackBottom - containerTop));
}

/** Where a jump scrolls the list to. `elementTop` is the row's top in the
 *  list's own scroll space. `inset` is the height at the top of the list the
 *  floating stack covers: the row is centered (or started) in what is left
 *  below it, and a row hiding under it counts as out of view. */
export function jumpTarget(o: {
  align: 'start' | 'center' | 'end' | 'auto';
  elementTop: number;
  elementHeight: number;
  scrollTop: number;
  containerHeight: number;
  scrollHeight: number;
  inset: number;
}): number {
  const inset = Math.max(0, Math.min(o.inset, o.containerHeight));
  const visible = o.containerHeight - inset;
  let target: number;
  if (o.align === 'center') {
    target = o.elementTop - inset - visible / 2 + o.elementHeight / 2;
  } else if (o.align === 'start') {
    target = o.elementTop - inset;
  } else if (o.align === 'end') {
    target = o.elementTop - o.containerHeight + o.elementHeight;
  } else {
    const top = o.elementTop - o.scrollTop;
    if (top < inset) target = o.elementTop - inset;
    else if (top + o.elementHeight > o.containerHeight) target = o.elementTop - o.containerHeight + o.elementHeight;
    else target = o.scrollTop;
  }
  return Math.max(0, Math.min(target, o.scrollHeight - o.containerHeight));
}

/** The breathing room under the stack before the first row of the list. */
export const STACK_BOTTOM_GAP_PX = 4;

/** The stack's drawn bottom, in the host's pixels, from where it starts and
 *  its height; 0 when it draws nothing. */
export function stackBottomOf(top: number, height: number): number {
  return height > 0 ? Math.round(top + height + STACK_BOTTOM_GAP_PX) : 0;
}
