// Where every MultiNook tile goes, for the stage's live size.
//
// One engine for every layout, all absolute rectangles: Grid (every tile the
// same size, the short row on top), and the main layouts (the first tile large,
// the rest small in a strip below or beside it). Tiles are positioned, never
// re-ordered in the DOM, so changing layout or swapping the main tile is a
// restyle and no player is rebuilt.
//
// This runs in the page because its input is the stage's pixel size from a
// ResizeObserver, every frame of a resize, and its output is only CSS. The
// chosen layout itself is a setting Rust keeps (settings.multi_nook_layout).

import type { MultiNookLayoutMode } from '../../types';

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** How much tile chrome fits: everything, a trimmed set, or the least. */
export type SizeTier = 'full' | 'compact' | 'mini';

export interface TilePlace extends Rect {
  tier: SizeTier;
  /** The large tile of a main layout. */
  main: boolean;
}

const RATIO = 16 / 9;
export const GAP = 8;
/** Below these widths the tile chrome trims down. */
export const COMPACT_BELOW = 360;
export const MINI_BELOW = 220;

export function tierFor(width: number): SizeTier {
  if (width < MINI_BELOW) return 'mini';
  if (width < COMPACT_BELOW) return 'compact';
  return 'full';
}

/** The largest 16:9 box that fits in `w` x `h`. */
function fit(w: number, h: number): { w: number; h: number } {
  return w / h > RATIO ? { w: h * RATIO, h } : { w, h: w / RATIO };
}

/**
 * `n` equal 16:9 tiles in a box: the column count with the largest tile wins,
 * rows are centred, and a row that is not full sits on top (so an odd count
 * reads as a pyramid). The grid MultiNook has always drawn, as rectangles.
 */
export function gridRects(box: Rect, n: number, gap = GAP): Rect[] {
  if (n <= 0 || box.w <= 0 || box.h <= 0) return [];
  let best = { area: 0, w: 0, h: 0, cols: 1 };
  for (let cols = 1; cols <= n; cols++) {
    const rows = Math.ceil(n / cols);
    let w = (box.w - (cols - 1) * gap) / cols;
    let h = w / RATIO;
    if (h * rows + (rows - 1) * gap > box.h) {
      h = (box.h - (rows - 1) * gap) / rows;
      w = h * RATIO;
    }
    if (w > 0 && h > 0 && w * h > best.area) best = { area: w * h, w, h, cols };
  }
  const w = Math.floor(best.w);
  const h = Math.floor(best.h);
  const cols = best.cols;
  const rows = Math.ceil(n / cols);
  const short = n % cols;
  const rowSizes = Array.from({ length: rows }, (_, r) => (r === 0 && short > 0 ? short : cols));
  const blockH = rows * h + (rows - 1) * gap;
  let y = box.y + (box.h - blockH) / 2;
  const out: Rect[] = [];
  for (const k of rowSizes) {
    const rowW = k * w + (k - 1) * gap;
    let x = box.x + (box.w - rowW) / 2;
    for (let i = 0; i < k; i++) {
      out.push({ x: Math.round(x), y: Math.round(y), w, h });
      x += w + gap;
    }
    y += h + gap;
  }
  return out;
}

/**
 * Rectangles for `count` visible tiles in a `width` x `height` stage. In a
 * main layout the first tile is large and `share` (0.15 to 0.40) of the stage
 * goes to the strip of small tiles: its height below the main tile, its width
 * beside it. The strip is filled by the grid solver, so any number of small
 * tiles fits without scrolling. The whole arrangement is centred.
 */
export function nookLayout(
  width: number,
  height: number,
  count: number,
  mode: MultiNookLayoutMode,
  share: number,
  gap = GAP,
): TilePlace[] {
  const stage: Rect = { x: 0, y: 0, w: width, h: height };
  const place = (r: Rect, main = false): TilePlace => ({ ...r, tier: tierFor(r.w), main });
  if (count <= 0 || width <= 0 || height <= 0) return [];
  // An unknown mode (a preset from a newer build) reads as Grid, as Rust reads it.
  if ((mode !== 'main_row' && mode !== 'main_column') || count === 1) {
    return gridRects(stage, count, gap).map((r) => place(r));
  }

  const s = Math.min(0.4, Math.max(0.15, Number.isFinite(share) ? share : 0.25));
  const small = count - 1;

  const asGrid = () => gridRects(stage, count, gap).map((r) => place(r));

  if (mode === 'main_row') {
    const stripH = Math.round(height * s);
    // A stage too small for a main tile and a strip (mid-collapse, say).
    if (height - stripH - gap <= 0 || stripH <= 0) return asGrid();
    const main = fit(width, height - stripH - gap);
    const mw = Math.floor(main.w);
    const mh = Math.floor(main.h);
    const top = (height - (mh + gap + stripH)) / 2;
    const mainRect: Rect = { x: Math.round((width - mw) / 2), y: Math.round(top), w: mw, h: mh };
    const strip: Rect = { x: 0, y: Math.round(top + mh + gap), w: width, h: stripH };
    return [place(mainRect, true), ...gridRects(strip, small, gap).map((r) => place(r))];
  }

  // main_column
  const stripW = Math.round(width * s);
  if (width - stripW - gap <= 0 || stripW <= 0) return asGrid();
  const main = fit(width - stripW - gap, height);
  const mw = Math.floor(main.w);
  const mh = Math.floor(main.h);
  const left = (width - (mw + gap + stripW)) / 2;
  const mainRect: Rect = { x: Math.round(left), y: Math.round((height - mh) / 2), w: mw, h: mh };
  const strip: Rect = { x: Math.round(left + mw + gap), y: 0, w: stripW, h: height };
  return [place(mainRect, true), ...gridRects(strip, small, gap).map((r) => place(r))];
}
