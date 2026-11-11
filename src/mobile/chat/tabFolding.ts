// How the room strip fits its rooms: the iPhone strip's rule (ChatTabStrip.swift,
// its ViewThatFits), so both phones compress the same way. The row never
// scrolls, clips or wraps. Rooms behind the one on screen fold to their faces
// one at a time, the longest name first, until the row fits; then the folded
// faces shrink and draw closer; then the room on screen drops its broadcast
// mark; then its name truncates, but only while a readable part of it stays
// (NAME_STUB). Narrower than that (the side chat at its smallest), the room on
// screen is its face alone, never a name cut to a letter or two.

/** The rooms' gap, and the tightest row's. */
export const TAB_GAP = 4;
export const TAB_GAP_TIGHT = 2;

/** The least of a name the room on screen shows cut, about seven letters. */
export const NAME_STUB = 56;

/** One room's widths in each form, as measured. */
export interface FoldTab {
  channel: string;
  label: string;
  /** Unfolded, its name capped. */
  full: number;
  /** Unfolded with no cap on the name (the room on screen, once others fold for it). */
  uncapped: number;
  /** Unfolded with no broadcast mark and its name capped at NAME_STUB. */
  stub: number;
  folded: number;
  foldedTight: number;
}

export interface Folding {
  /** The rooms showing only their faces. */
  folded: Set<string>;
  /** The tightest row: folded faces smaller and closer. */
  tight: boolean;
  /** The room on screen leaves out its broadcast mark. */
  bare: boolean;
}

/** The row's width with the given rooms folded; `stub` measures the room on
 *  screen in its stub form. */
export function rowWidth(
  tabs: FoldTab[],
  active: string | null,
  folded: Set<string>,
  tight: boolean,
  stub = false,
): number {
  const crowded = folded.size > 0;
  let width = 0;
  for (const t of tabs) {
    if (folded.has(t.channel)) width += tight ? t.foldedTight : t.folded;
    else if (t.channel === active && stub) width += t.stub;
    else width += crowded && t.channel === active ? t.uncapped : t.full;
  }
  return width + (tight ? TAB_GAP_TIGHT : TAB_GAP) * Math.max(0, tabs.length - 1);
}

/** No room folded, then one more at a time (rooms not on screen, longest name
 *  first), then the tightest row, then the tightest row with the room on
 *  screen bare and its name cut no shorter than NAME_STUB: the first that fits
 *  `available` wins. When none does, every room is a face, the one on screen
 *  included. */
export function pickFolding(tabs: FoldTab[], active: string | null, available: number): Folding {
  const order = tabs
    .filter((t) => t.channel !== active)
    .sort((a, b) => b.label.length - a.label.length);
  const foldedBy = (n: number) => new Set(order.slice(0, n).map((t) => t.channel));
  for (let n = 0; n < Math.max(1, tabs.length); n++) {
    const folded = foldedBy(n);
    if (rowWidth(tabs, active, folded, false) <= available) return { folded, tight: false, bare: false };
  }
  const others = foldedBy(Math.max(0, tabs.length - 1));
  if (rowWidth(tabs, active, others, true) <= available) return { folded: others, tight: true, bare: false };
  if (rowWidth(tabs, active, others, true, true) <= available) return { folded: others, tight: true, bare: true };
  return { folded: new Set(tabs.map((t) => t.channel)), tight: true, bare: true };
}
