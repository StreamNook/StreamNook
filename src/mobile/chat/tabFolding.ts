// How the room strip fits its rooms: the iPhone strip's rule (ChatTabStrip.swift,
// its ViewThatFits), so both phones compress the same way. The row never
// scrolls, clips or wraps. Rooms behind the one on screen fold to their faces
// one at a time, the longest name first, until the row fits; then the folded
// faces shrink and draw closer; only then does the name on screen truncate.

/** The rooms' gap, and the tightest row's. */
export const TAB_GAP = 4;
export const TAB_GAP_TIGHT = 2;

/** One room's widths in each form, as measured. */
export interface FoldTab {
  channel: string;
  label: string;
  /** Unfolded, its name capped. */
  full: number;
  /** Unfolded with no cap on the name (the room on screen, once others fold for it). */
  uncapped: number;
  folded: number;
  foldedTight: number;
}

export interface Folding {
  /** The rooms showing only their faces. */
  folded: Set<string>;
  /** The tightest row: folded faces smaller and closer. */
  tight: boolean;
}

/** The row's width with the given rooms folded. */
export function rowWidth(tabs: FoldTab[], active: string | null, folded: Set<string>, tight: boolean): number {
  const crowded = folded.size > 0;
  let width = 0;
  for (const t of tabs) {
    if (folded.has(t.channel)) width += tight ? t.foldedTight : t.folded;
    else width += crowded && t.channel === active ? t.uncapped : t.full;
  }
  return width + (tight ? TAB_GAP_TIGHT : TAB_GAP) * Math.max(0, tabs.length - 1);
}

/** No room folded, then one more at a time (rooms not on screen, longest name
 *  first), then the tightest row: the first that fits `available` wins. When
 *  none does, the tightest row, where the name on screen gives way. */
export function pickFolding(tabs: FoldTab[], active: string | null, available: number): Folding {
  const order = tabs
    .filter((t) => t.channel !== active)
    .sort((a, b) => b.label.length - a.label.length);
  const foldedBy = (n: number) => new Set(order.slice(0, n).map((t) => t.channel));
  for (let n = 0; n < Math.max(1, tabs.length); n++) {
    const folded = foldedBy(n);
    if (rowWidth(tabs, active, folded, false) <= available) return { folded, tight: false };
  }
  return { folded: foldedBy(Math.max(0, tabs.length - 1)), tight: true };
}
