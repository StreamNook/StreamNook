// The cover wash must keep every muted word as readable as the theme makes it
// (4.5:1 at most) wherever it is drawn: on the surface as it is really painted
// (the flyout's tertiary base and secondary layer, with and without its top
// light, with and without a hovered tile), at every checked point, read back
// from the CSS string the page actually emits. The compositor below is this
// test's own, so a wrong layer order, size or alpha in that string fails.

import { readFileSync } from 'node:fs';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import builtIn from '../themes/builtinThemes.json';
import { getOledTheme, opaqueFloors, phoneFloors, WINDOW_FLOOR, type Theme } from '../themes';
import {
  CARD_LAYERS,
  FIT_SETTLE_MS,
  fitSize,
  fitTint,
  makeSurface,
  PANEL_LAYERS,
  parseBackground,
  parseColor,
  rowTint,
  TILE_LIGHT,
  TOP_LIGHT,
  useCoverFit,
  washBackground,
  type CoverTint,
  type LayerDef,
  type ParsedLayer,
  type Rgb,
  type Surface,
  type SurfaceKind,
} from './coverTint';

// ---------------------------------------------------------------- an independent compositor

type C = [number, number, number];
const lin = (c: number) => {
  const v = c / 255;
  return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
};
const lum = ([r, g, b]: C) => 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
const ratio = (a: C, b: C) => {
  const x = lum(a);
  const y = lum(b);
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
};
const mix = (top: C, a: number, under: C): C => [0, 1, 2].map((i) => under[i] + (top[i] - under[i]) * a) as C;
const WHITE: C = [255, 255, 255];

/** The parsed background at (x, y), px from its box's top left, over `under`:
 *  the last layer is drawn first (CSS puts the first on top). */
function paint(layers: ParsedLayer[], x: number, y: number, under: C): C {
  let c = under;
  for (const l of [...layers].reverse()) {
    if (l.kind === 'fill') c = mix(l.rgb, l.alpha, c);
    else if (l.kind === 'radial') {
      expect(l.unit).toBe('px');
      const d = Math.hypot((x - l.cx) / l.rx, (y - l.cy) / l.ry);
      c = mix(l.rgb, l.alpha + (l.alphaEnd - l.alpha) * Math.min(1, d), c);
    }
  }
  return c;
}

const mutedOver = (s: Surface, c: C) => ratio(mix([s.muted[0], s.muted[1], s.muted[2]], s.muted[3], c), c);

/** The four bases a point is read on: the plain surface with and without the
 *  top light, each with and without a hovered tile over the wash. */
function bases(s: Surface): { under: C; tile: number }[] {
  const plain: C = s.secondary
    ? mix([s.secondary[0], s.secondary[1], s.secondary[2]], s.secondary[3], s.base)
    : [...s.base];
  const lit = mix(WHITE, TOP_LIGHT, plain);
  return [
    { under: plain, tile: 0 },
    { under: lit, tile: 0 },
    { under: plain, tile: TILE_LIGHT },
    { under: lit, tile: TILE_LIGHT },
  ];
}

/** The checked points: a grid every 20 px with the far edges and corners, and
 *  each layer's center brought inside. */
function points(w: number, h: number, layers: LayerDef[]): [number, number][] {
  const axis = (len: number) => {
    const out: number[] = [];
    for (let v = 0; v < len; v += 20) out.push(v);
    out.push(len);
    return out;
  };
  const out: [number, number][] = [];
  for (const x of axis(w)) for (const y of axis(h)) out.push([x, y]);
  for (const l of layers) out.push([Math.min(1, Math.max(0, l.cx)) * w, Math.min(1, Math.max(0, l.cy)) * h]);
  return out;
}

// ---------------------------------------------------------------- the five themes

const theme = (id: string): Theme => {
  if (id === 'prince0fdubai-oled') return getOledTheme('#22d3ee');
  const t = (builtIn.themes as unknown as Theme[]).find((x) => x.id === id);
  if (!t) throw new Error(id);
  return t;
};

/** A theme's surface as the page paints it, through the app's own floors. */
function surfaceOf(id: string, kind: SurfaceKind): Surface {
  const p = theme(id).palette;
  const floors = kind === 'flyout' ? opaqueFloors(p.background, p.backgroundTertiary, WINDOW_FLOOR) : phoneFloors(p.background, p.backgroundTertiary);
  const base = parseColor(kind === 'flyout' ? floors.tertiary : floors.background)!;
  return makeSurface(kind, [base[0], base[1], base[2]], parseColor(p.backgroundSecondary), parseColor(p.textMuted)!);
}

const THEMES = ['winters-glass', 'catppuccin-latte', 'solarized-sand', 'dracula', 'prince0fdubai-oled'];
const TINTS: [string, CoverTint][] = [
  ['vivid', { main: [0.6, 0.2, 25], second: [0.7, 0.12, 250] }],
  ['pale', { main: [0.82, 0.03, 75], second: null }],
  ['deep blue', { main: [0.3, 0.12, 260], second: [0.8, 0.1, 90] }],
];

describe('the wash keeps the muted words readable', () => {
  for (const id of THEMES) {
    for (const kind of ['flyout', 'screen'] as const) {
      it(`${id} (${kind})`, () => {
        const s = surfaceOf(id, kind);
        const [w, h] = kind === 'flyout' ? [520, 620] : [390, 780];
        for (const [name, tint] of TINTS) {
          const fit = fitTint(tint, s, w, h)!;
          expect(fit, name).not.toBeNull();
          const layers = parseBackground(washBackground(fit, { w, h }, { x: 0, y: 0, w, h }));
          for (const b of bases(s)) {
            const shown = (c: C) => (b.tile ? mix(WHITE, b.tile, c) : c);
            const need = Math.min(4.5, mutedOver(s, shown(b.under)));
            for (const [x, y] of points(w, h, PANEL_LAYERS)) {
              const got = mutedOver(s, shown(paint(layers, x, y, b.under)));
              if (got < need - 1e-6) throw new Error(`${id} ${name} at ${x},${y}: ${got.toFixed(4)} < ${need.toFixed(4)}`);
            }
          }
        }
      });
    }
  }

  it('tints every one of the five themes', () => {
    for (const id of THEMES) {
      const fit = fitTint(TINTS[0][1], surfaceOf(id, 'flyout'), 520, 620)!;
      expect(fit.k, id).toBeGreaterThan(0.3);
    }
  });

  it('keeps the ink at 4.5:1 on the wash', () => {
    for (const id of THEMES) {
      const s = surfaceOf(id, 'flyout');
      for (const [, tint] of TINTS) {
        const fit = fitTint(tint, s, 520, 620)!;
        const layers = parseBackground(washBackground(fit, { w: 520, h: 620 }, { x: 0, y: 0, w: 520, h: 620 }));
        const worst = Math.min(
          ...bases(s)
            .filter((b) => b.tile === 0)
            .flatMap((b) => points(520, 620, PANEL_LAYERS).map(([x, y]) => ratio(fit.inkRgb, paint(layers, x, y, b.under)))),
        );
        expect(worst, id).toBeGreaterThanOrEqual(4.5 - 1e-6);
        expect(fit.ink).toBe(`rgb(${fit.inkRgb.join(', ')})`);
      }
    }
  });

  it('keeps the Latest card readable with its tile under the wash', () => {
    for (const id of THEMES) {
      const s = surfaceOf(id, 'flyout');
      const fit = fitTint(TINTS[0][1], s, 488, 112, CARD_LAYERS, { onTile: true })!;
      const layers = parseBackground(washBackground(fit, { w: 488, h: 112 }, { x: 0, y: 0, w: 488, h: 112 }));
      for (const b of bases(s)) {
        const under = b.tile ? mix(WHITE, b.tile, b.under) : b.under;
        const need = Math.min(4.5, mutedOver(s, under));
        for (const [x, y] of points(488, 112, CARD_LAYERS)) {
          expect(mutedOver(s, paint(layers, x, y, under))).toBeGreaterThanOrEqual(need - 1e-6);
        }
      }
    }
  });
});

describe('the emitted background', () => {
  const s = surfaceOf('winters-glass', 'flyout');
  const fit = fitTint(TINTS[0][1], s, 520, 620)!;

  it('leaves a gray cover, or none, exactly as the theme paints it', () => {
    expect(fitTint({ main: null, second: null }, s, 520, 620)).toBeNull();
    expect(fitTint(null, s, 520, 620)).toBeNull();
    expect(fitTint(undefined, s, 520, 620)).toBeNull();
    expect(rowTint({ main: null, second: null }, s)).toBeNull();
  });

  it('lists the layers second, main, deep, then the fill, at their sizes', () => {
    const layers = parseBackground(washBackground(fit, { w: 520, h: 620 }, { x: 0, y: 0, w: 520, h: 620 }));
    expect(layers.map((l) => l.kind)).toEqual(['radial', 'radial', 'radial', 'fill']);
    const [second, main, deep, fill] = layers as Extract<ParsedLayer, { kind: 'radial' }>[];
    expect(second.rgb).toEqual(fit.colors.second);
    expect(main.rgb).toEqual(fit.colors.main);
    expect(deep.rgb).toEqual(fit.colors.deep);
    expect((fill as unknown as { rgb: Rgb }).rgb).toEqual(fit.colors.main);
    expect([second.rx, second.ry, second.cx, second.cy]).toEqual([0.8 * 520, 0.55 * 620, 0.96 * 520, 0.02 * 620]);
    expect([main.rx, main.ry, main.cx, main.cy]).toEqual([1.05 * 520, 0.7 * 620, 0.06 * 520, -0.02 * 620]);
    expect([deep.rx, deep.ry, deep.cx, deep.cy]).toEqual([1.2 * 520, 0.65 * 620, 0.5 * 520, 1.1 * 620]);
    expect(second.alphaEnd).toBe(0);
  });

  it('lines a sub-box up with the whole (an edge fade paints what the panel does)', () => {
    const whole = parseBackground(washBackground(fit, { w: 520, h: 620 }, { x: 0, y: 0, w: 520, h: 620 }));
    const box = { x: 0, y: 580, w: 520, h: 40 };
    const strip = parseBackground(washBackground(fit, { w: 520, h: 620 }, box));
    const under: C = [32, 32, 33];
    for (const [x, y] of [
      [0, 0],
      [260, 20],
      [519, 39],
      [130, 7],
    ]) {
      const a = paint(whole, x + box.x, y + box.y, under);
      const b = paint(strip, x, y, under);
      for (let i = 0; i < 3; i++) expect(b[i]).toBeCloseTo(a[i], 2);
    }
  });

  it('takes the tile and top-light alphas from globals.css', () => {
    const css = readFileSync(new URL('../styles/globals.css', import.meta.url), 'utf8');
    const hover = /\.glass-tile:hover\s*\{[^}]*?linear-gradient\(180deg,\s*rgba\(255,\s*255,\s*255,\s*([\d.]+)\)/.exec(css);
    expect(Number(hover?.[1])).toBe(TILE_LIGHT);
    const flyout = /\n\.glass-flyout\s*\{([\s\S]*?)\n\}/.exec(css)?.[1] ?? '';
    const light = /background-image:\s*linear-gradient\(180deg,\s*rgba\(255,\s*255,\s*255,\s*([\d.]+)\)/.exec(flyout);
    expect(Number(light?.[1])).toBe(TOP_LIGHT);
  });
});

describe('what a fit costs', () => {
  it('stays within its composite bound for a 520 x 620 surface', () => {
    const w = 520;
    const h = 620;
    const n = points(w, h, PANEL_LAYERS).length;
    for (const id of THEMES) {
      for (const [, tint] of TINTS) {
        let checks = 0;
        fitTint(tint, surfaceOf(id, 'flyout'), w, h, PANEL_LAYERS, { count: (c) => (checks = c) });
        expect(checks).toBeGreaterThan(0);
        expect(checks).toBeLessThanOrEqual(n * 4 * 18 + n * 40);
      }
    }
  });

  it('renders as server markup with the plain theme (no page to read the theme from)', () => {
    function Probe() {
      const fit = useCoverFit(TINTS[0][1], 'flyout', 520, 620);
      return createElement('span', null, fit === null ? 'plain' : 'washed');
    }
    expect(renderToStaticMarkup(createElement(Probe))).toBe('<span>plain</span>');
  });

  it('fits a new size only once it has held for the settle time', () => {
    expect(FIT_SETTLE_MS).toBe(150);
    const shown = { w: 520, h: 620 };
    // The first real size fits at once.
    expect(fitSize({ w: 0, h: 0 }, shown)).toEqual({ size: shown, wait: false });
    // A resize under way keeps the last fit's size (its layers are fractions).
    const resizing = fitSize(shown, { w: 540, h: 620 });
    expect(resizing.wait).toBe(true);
    expect(resizing.size).toBe(shown);
    expect(fitSize(shown, { w: 520, h: 620 })).toEqual({ size: shown, wait: false });
  });
});

describe('a row hover tint', () => {
  it("keeps the muted words, or falls back to the theme's own hover", () => {
    for (const id of THEMES) {
      const s = surfaceOf(id, 'flyout');
      const [plain, lit] = bases(s);
      for (const [, tint] of TINTS) {
        const tinted = rowTint(tint, s);
        if (!tinted) continue;
        const c = parseColor(tinted)!;
        for (const b of [plain, lit]) {
          const need = Math.min(4.5, mutedOver(s, b.under));
          expect(mutedOver(s, mix([c[0], c[1], c[2]], c[3], b.under))).toBeGreaterThanOrEqual(need - 1e-6);
        }
      }
    }
  });

  it('is as strong as reads on a light theme', () => {
    expect(rowTint(TINTS[0][1], surfaceOf('catppuccin-latte', 'flyout'))).toMatch(/, 0\.6\)$/);
  });

  it('is none when no alpha keeps the muted words', () => {
    // Dracula's muted words sit under 4.5:1 on its flyout already, so a lighter
    // row costs them contrast at any alpha.
    const s = surfaceOf('dracula', 'flyout');
    for (const [, tint] of TINTS) expect(rowTint(tint, s)).toBeNull();
  });
});

describe('reading colors', () => {
  it('reads computed and authored colors', () => {
    expect(parseColor('rgb(26, 26, 27)')).toEqual([26, 26, 27, 1]);
    expect(parseColor('rgba(255, 255, 255, 0.03)')).toEqual([255, 255, 255, 0.03]);
    expect(parseColor('rgb(10 20 30 / 50%)')).toEqual([10, 20, 30, 0.5]);
    expect(parseColor('#1a1a1b')).toEqual([26, 26, 27, 1]);
    expect(parseColor('transparent')).toEqual([0, 0, 0, 0]);
    expect(parseColor('color-mix(in srgb, red, blue)')).toBeNull();
  });
});
