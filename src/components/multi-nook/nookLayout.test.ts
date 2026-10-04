import { describe, expect, it } from 'vitest';
import { GAP, gridRects, nookLayout, tierFor, type Rect } from './nookLayout';

// The flex grid MultiNookView drew before the layout engine: the same solver,
// returning the cell size and column count. Grid must keep producing exactly
// these cells.
function legacyCell(W: number, H: number, len: number) {
  let best = { area: 0, w: 0, h: 0, cols: 1 };
  for (let cols = 1; cols <= len; cols++) {
    const rows = Math.ceil(len / cols);
    let w = (W - (cols - 1) * GAP) / cols;
    let h = w / (16 / 9);
    if (h * rows + (rows - 1) * GAP > H) {
      h = (H - (rows - 1) * GAP) / rows;
      w = h * (16 / 9);
    }
    if (w * h > best.area) best = { area: w * h, w, h, cols };
  }
  return { w: Math.floor(best.w), h: Math.floor(best.h), cols: best.cols };
}

const overlaps = (a: Rect, b: Rect) => a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
const inside = (r: Rect, W: number, H: number) => r.x >= 0 && r.y >= 0 && r.x + r.w <= W + 0.5 && r.y + r.h <= H + 0.5;
const isWide = (r: Rect) => Math.abs(r.w / r.h - 16 / 9) < 0.05;

describe('Grid', () => {
  for (const [W, H] of [[1400, 860], [1100, 900], [800, 450], [1920, 1000]] as const) {
    for (let n = 1; n <= 25; n++) {
      it(`keeps the old cells for ${n} tiles at ${W}x${H}`, () => {
        const rects = nookLayout(W, H, n, 'grid', 0.25);
        const legacy = legacyCell(W, H, n);
        expect(rects).toHaveLength(n);
        for (const r of rects) {
          expect(r.w).toBe(legacy.w);
          expect(r.h).toBe(legacy.h);
          expect(inside(r, W, H)).toBe(true);
          expect(r.main).toBe(false);
        }
        for (let i = 0; i < n; i++) for (let j = i + 1; j < n; j++) expect(overlaps(rects[i], rects[j])).toBe(false);
      });
    }
  }

  it('puts the short row on top', () => {
    // 5 tiles in a wide box solve to 3 columns: 2 on top, 3 below.
    const rects = gridRects({ x: 0, y: 0, w: 1500, h: 600 }, 5);
    const rows = [...new Set(rects.map((r) => r.y))].sort((a, b) => a - b);
    expect(rows).toHaveLength(2);
    expect(rects.filter((r) => r.y === rows[0])).toHaveLength(2);
    expect(rects.filter((r) => r.y === rows[1])).toHaveLength(3);
  });
});

describe('main layouts', () => {
  for (const mode of ['main_row', 'main_column'] as const) {
    for (const share of [0.15, 0.25, 0.4]) {
      for (const [W, H] of [[1400, 860], [1100, 900], [1920, 1000]] as const) {
        it(`${mode} at ${share} fits 1 to 25 tiles at ${W}x${H}`, () => {
          for (let n = 1; n <= 25; n++) {
            const rects = nookLayout(W, H, n, mode, share);
            expect(rects).toHaveLength(n);
            for (const r of rects) {
              expect(inside(r, W, H)).toBe(true);
              expect(isWide(r)).toBe(true);
            }
            for (let i = 0; i < n; i++) for (let j = i + 1; j < n; j++) expect(overlaps(rects[i], rects[j])).toBe(false);
            if (n > 1) {
              expect(rects[0].main).toBe(true);
              for (const r of rects.slice(1)) {
                expect(r.main).toBe(false);
                expect(r.w).toBeLessThan(rects[0].w);
              }
            }
          }
        });
      }
    }
  }

  it('puts the strip below the main tile, or beside it', () => {
    const row = nookLayout(1400, 860, 5, 'main_row', 0.25);
    for (const r of row.slice(1)) expect(r.y).toBeGreaterThanOrEqual(row[0].y + row[0].h);
    const col = nookLayout(1400, 860, 5, 'main_column', 0.25);
    for (const r of col.slice(1)) expect(r.x).toBeGreaterThanOrEqual(col[0].x + col[0].w);
  });

  it('gives the strip more room as the share grows', () => {
    const small = nookLayout(1400, 860, 5, 'main_row', 0.15)[1];
    const large = nookLayout(1400, 860, 5, 'main_row', 0.4)[1];
    expect(large.w).toBeGreaterThan(small.w);
  });

  it('one tile fills the stage in any mode', () => {
    expect(nookLayout(1400, 860, 1, 'main_row', 0.25)).toEqual(nookLayout(1400, 860, 1, 'grid', 0.25));
  });

  it('a share out of range is held to 15 to 40 percent', () => {
    expect(nookLayout(1400, 860, 4, 'main_row', 9)).toEqual(nookLayout(1400, 860, 4, 'main_row', 0.4));
    expect(nookLayout(1400, 860, 4, 'main_row', Number.NaN)).toEqual(nookLayout(1400, 860, 4, 'main_row', 0.25));
  });

  it('an unknown mode lays out as Grid', () => {
    const unknown = 'hexagon_swirl' as unknown as 'grid';
    expect(nookLayout(1400, 860, 5, unknown, 0.25)).toEqual(nookLayout(1400, 860, 5, 'grid', 0.25));
  });

  it('a stage too small for a main tile and a strip falls back to Grid', () => {
    for (const mode of ['main_row', 'main_column'] as const) {
      const rects = nookLayout(12, 9, 4, mode, 0.4);
      for (const r of rects) {
        expect(r.w).toBeGreaterThanOrEqual(0);
        expect(r.h).toBeGreaterThanOrEqual(0);
      }
      expect(rects).toEqual(nookLayout(12, 9, 4, 'grid', 0.4));
    }
  });

  it('a stage with no size has no tiles', () => {
    expect(nookLayout(0, 860, 4, 'main_row', 0.25)).toEqual([]);
  });
});

describe('tierFor', () => {
  it('trims the chrome as tiles shrink', () => {
    expect(tierFor(600)).toBe('full');
    expect(tierFor(300)).toBe('compact');
    expect(tierFor(180)).toBe('mini');
  });
});
