// Cover colors fitted to the live theme. Rust keeps each cover's colors
// (services/song_id/cover_tint.rs): OKLCH `main` and `second`, or `main: null`
// for a gray cover. This fits them to the surface a song is drawn on:
//
// - a wash of the cover's hue (not its light) over the surface, every layer
//   scaled by one factor `k`, the most that keeps every muted word as readable
//   as the theme makes it, at every checked point of the surface as it is
//   really painted (the flyout's tertiary base, its secondary layer, its top
//   light, a hovered tile);
// - the cover's ink, its hue at a lightness that reads 4.5:1 everywhere on
//   the wash, for accent words and marks;
// - a list row's hover tint, or none when no tint keeps the muted words.
//
// The heavy part (the cover's swatches) ran once in Rust. This is styling,
// so it lives where the theme's resolved CSS lives, and it is cheap: per size
// the points and each layer's falloff at them are computed once, and a fit is
// then only compositing, bounded by an operation count.

import { useEffect, useMemo, useState, useSyncExternalStore } from 'react';
import { subscribeTheme, themeVersion } from '../themes';

/** OKLCH lightness (0 to 1), chroma and hue (degrees). */
export type Lch = [number, number, number];
/** sRGB channels, 0 to 255. */
export type Rgb = [number, number, number];
/** sRGB channels 0 to 255, alpha 0 to 1. */
export type Rgba = [number, number, number, number];

/** A cover's colors as Rust sends them; `main: null` is a gray cover. */
export interface CoverTint {
  main: Lch | null;
  second: Lch | null;
}

// ---------------------------------------------------------------- color math

const toLinExact = (v: number): number => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
const toEnc = (v: number): number => (v <= 0.0031308 ? 12.92 * v : 1.055 * v ** (1 / 2.4) - 0.055);

// Linear light of an encoded channel (0 to 255, fractional allowed), from a
// table: a fit reads tens of thousands of luminances.
const LIN_STEPS = 4096;
const LIN = new Float64Array(LIN_STEPS + 1);
for (let i = 0; i <= LIN_STEPS; i++) LIN[i] = toLinExact(i / LIN_STEPS);
function toLin(c255: number): number {
  const x = (Math.min(255, Math.max(0, c255)) / 255) * LIN_STEPS;
  const i = Math.min(LIN_STEPS - 1, Math.floor(x));
  return LIN[i] + (LIN[i + 1] - LIN[i]) * (x - i);
}

/** WCAG relative luminance. */
export function luminance(r: number, g: number, b: number): number {
  return 0.2126 * toLin(r) + 0.7152 * toLin(g) + 0.0722 * toLin(b);
}

/** WCAG contrast ratio of two luminances. */
const contrastOf = (x: number, y: number): number => (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);

/** WCAG contrast ratio of two colors. */
export function contrast(a: Rgb, b: Rgb): number {
  return contrastOf(luminance(a[0], a[1], a[2]), luminance(b[0], b[1], b[2]));
}

/** `top` at alpha `a` painted over the opaque `under` (source-over). */
export function over(top: Rgb, a: number, under: Rgb): Rgb {
  return [under[0] + (top[0] - under[0]) * a, under[1] + (top[1] - under[1]) * a, under[2] + (top[2] - under[2]) * a];
}

/** OKLCH of an sRGB color. */
export function lchOf([r8, g8, b8]: Rgb): Lch {
  const r = toLinExact(r8 / 255);
  const g = toLinExact(g8 / 255);
  const b = toLinExact(b8 / 255);
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  const L = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
  const A = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
  const B = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
  const h = (Math.atan2(B, A) * 180) / Math.PI;
  return [L, Math.hypot(A, B), h < 0 ? h + 360 : h];
}

function linRgb(L: number, c: number, h: number): [number, number, number] {
  const a = c * Math.cos((h * Math.PI) / 180);
  const b = c * Math.sin((h * Math.PI) / 180);
  const l1 = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m1 = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s1 = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
  return [
    4.0767416621 * l1 - 3.3077115913 * m1 + 0.2309699292 * s1,
    -1.2684380046 * l1 + 2.6097574011 * m1 - 0.3413193965 * s1,
    -0.0041960863 * l1 - 0.7034186147 * m1 + 1.707614701 * s1,
  ];
}

const inGamut = (v: number[]): boolean => v.every((x) => x >= -1e-4 && x <= 1.0001);

/** The sRGB color at this OKLCH, channels rounded; a chroma the screen cannot
 *  show is lowered until it can, keeping lightness and hue. */
export function fromLch(L: number, c: number, h: number): Rgb {
  const l = Math.min(1, Math.max(0, L));
  let v = linRgb(l, c, h);
  if (!inGamut(v)) {
    let lo = 0;
    let hi = c;
    for (let i = 0; i < 24; i++) {
      const mid = (lo + hi) / 2;
      if (inGamut(linRgb(l, mid, h))) lo = mid;
      else hi = mid;
    }
    v = linRgb(l, lo, h);
  }
  return v.map((x) => Math.round(toEnc(Math.min(1, Math.max(0, x))) * 255)) as Rgb;
}

/** A CSS color as rgba channels: `rgb()`/`rgba()` in either syntax, hex, or
 *  `transparent`. Null for anything else. */
export function parseColor(css: string): Rgba | null {
  const s = css.trim().toLowerCase();
  if (s === 'transparent') return [0, 0, 0, 0];
  const hex = /^#([0-9a-f]{3,8})$/.exec(s);
  if (hex) {
    let h = hex[1];
    if (h.length === 3 || h.length === 4) h = [...h].map((x) => x + x).join('');
    if (h.length !== 6 && h.length !== 8) return null;
    const n = (i: number) => parseInt(h.slice(i, i + 2), 16);
    return [n(0), n(2), n(4), h.length === 8 ? n(6) / 255 : 1];
  }
  const fn = /^rgba?\(([^)]*)\)$/.exec(s);
  if (!fn) return null;
  const parts = fn[1].split(/[\s,/]+/).filter(Boolean);
  if (parts.length < 3) return null;
  const channel = (p: string) => (p.endsWith('%') ? (parseFloat(p) / 100) * 255 : parseFloat(p));
  const alpha = parts[3] === undefined ? 1 : parts[3].endsWith('%') ? parseFloat(parts[3]) / 100 : parseFloat(parts[3]);
  const out: Rgba = [channel(parts[0]), channel(parts[1]), channel(parts[2]), alpha];
  return out.every((x) => Number.isFinite(x)) ? out : null;
}

const floor3 = (a: number): number => Math.max(0, Math.floor(a * 1000 + 1e-9) / 1000);
const num = (v: number): string => String(Math.round(v * 100) / 100);
const rgbCss = ([r, g, b]: Rgb): string => `rgb(${Math.round(r)}, ${Math.round(g)}, ${Math.round(b)})`;
const rgbaCss = ([r, g, b]: Rgb, a: number): string =>
  `rgba(${Math.round(r)}, ${Math.round(g)}, ${Math.round(b)}, ${floor3(a)})`;

const WHITE: Rgb = [255, 255, 255];

// ---------------------------------------------------------------- the surface

/** White over the top of a flyout: the first stop of `.glass-flyout`'s top
 *  light in globals.css (a test reads the file to keep them equal). */
export const TOP_LIGHT = 0.05;
/** White over a hovered `.glass-tile`: the first stop of its hover gradient
 *  in globals.css (the same test). */
export const TILE_LIGHT = 0.07;

/** Where a song is drawn: the desktop's flyout (the Your songs modal and the
 *  Songs popover) or the phone's screen. */
export type SurfaceKind = 'flyout' | 'screen';

/** A surface as it is painted, bottom up: an opaque `base`, the `secondary`
 *  layer over it, a white top light at `topLight` over the top part, and a
 *  tile's white at `tile` where a tile sits; `muted` is the theme's muted
 *  text with its alpha. */
export interface Surface {
  kind: SurfaceKind;
  base: Rgb;
  secondary: Rgba | null;
  topLight: number;
  tile: number;
  muted: Rgba;
  /** A light theme: the plain surface's luminance is over 0.4. */
  light: boolean;
}

/** The plain surface: the base with its secondary layer. */
const plainOf = (s: Surface): Rgb => (s.secondary ? over(rgbOf(s.secondary), s.secondary[3], s.base) : s.base);
const rgbOf = (c: Rgba): Rgb => [c[0], c[1], c[2]];

/** A surface from its painted colors, the way readSurface builds one. */
export function makeSurface(kind: SurfaceKind, base: Rgb, secondary: Rgba | null, muted: Rgba): Surface {
  const s: Surface = {
    kind,
    base,
    secondary: kind === 'flyout' ? secondary : null,
    topLight: kind === 'flyout' ? TOP_LIGHT : 0,
    tile: TILE_LIGHT,
    muted,
    light: false,
  };
  const plain = plainOf(s);
  s.light = luminance(plain[0], plain[1], plain[2]) > 0.4;
  return s;
}

// What sits under the page (themes/index.ts WINDOW_FLOOR): a base that probes
// see-through is read over it.
const UNDER: Rgb = [12, 12, 13];
const FALLBACK = makeSurface('flyout', [26, 26, 27], [255, 255, 255, 0.03], [151, 177, 185, 0.75]);

const surfaces = new Map<SurfaceKind, { version: number; surface: Surface }>();

/** The live theme's surface of this kind, read through probes once per theme
 *  version. */
export function readSurface(kind: SurfaceKind): Surface {
  const version = themeVersion();
  const kept = surfaces.get(kind);
  if (kept && kept.version === version) return kept.surface;
  if (typeof document === 'undefined' || !document.body) return FALLBACK;
  const probe = document.createElement('div');
  probe.setAttribute('aria-hidden', 'true');
  probe.style.cssText = 'position:absolute;width:0;height:0;overflow:hidden;visibility:hidden;pointer-events:none';
  document.body.appendChild(probe);
  const read = (prop: 'backgroundColor' | 'color', value: string): Rgba | null => {
    probe.style[prop] = value;
    return parseColor(getComputedStyle(probe)[prop]);
  };
  const baseRead = read('backgroundColor', kind === 'flyout' ? 'var(--color-background-tertiary)' : 'var(--color-background)');
  const secondary = kind === 'flyout' ? read('backgroundColor', 'var(--color-background-secondary)') : null;
  probe.style.backgroundColor = '';
  const muted = read('color', 'var(--color-text-muted)');
  probe.remove();
  const base: Rgb = baseRead ? over(rgbOf(baseRead), baseRead[3], UNDER) : FALLBACK.base;
  const surface = makeSurface(kind, base, secondary && secondary[3] > 0 ? secondary : null, muted ?? FALLBACK.muted);
  surfaces.set(kind, { version, surface });
  return surface;
}

// ---------------------------------------------------------------- the wash

/** One radial layer of a wash, in fractions of its surface: center, radii,
 *  its alpha at the center at k = 1, and which of the cover's colors. */
export interface LayerDef {
  cx: number;
  cy: number;
  rx: number;
  ry: number;
  most: number;
  color: 'main' | 'second' | 'deep';
}

/** The song page's wash, in CSS order (the first layer is drawn on top). */
export const PANEL_LAYERS: LayerDef[] = [
  { cx: 0.96, cy: 0.02, rx: 0.8, ry: 0.55, most: 0.7, color: 'second' },
  { cx: 0.06, cy: -0.02, rx: 1.05, ry: 0.7, most: 0.9, color: 'main' },
  { cx: 0.5, cy: 1.1, rx: 1.2, ry: 0.65, most: 0.8, color: 'deep' },
];

/** The Latest card's wash, in CSS order. The card is a tile, so its fit is
 *  asked with `{ onTile: true }`. */
export const CARD_LAYERS: LayerDef[] = [
  { cx: 0, cy: 0, rx: 0.9, ry: 1.6, most: 0.6, color: 'main' },
  { cx: 1, cy: 1, rx: 0.7, ry: 1.4, most: 0.45, color: 'second' },
];

/** The flat fill of the main color under the radial layers, at k = 1. */
export const FILL_MOST = 0.45;

/** The muted words keep this much contrast, or the theme's own when it has
 *  less. */
const NEED_MAX = 4.5;
/** The ink's contrast on every point of the wash. */
const INK_NEED = 4.5;
/** Checked points are this far apart. */
const GRID_PX = 20;
const K_CHECKS = 18;
const INK_CHECKS = 40;
/** A k must clear the theme's contrast by this much (the table's error). */
const MARGIN = 1e-4;

export interface Fit {
  /** The one scale every layer and the fill are drawn at. */
  k: number;
  colors: { main: Rgb; second: Rgb; deep: Rgb };
  layers: LayerDef[];
  /** The fill's alpha at k = 1. */
  fill: number;
  /** The cover's ink, as a CSS color. */
  ink: string;
  inkRgb: Rgb;
  /** The main color for a cover's shadow. */
  glow: string;
  /** A section mark in the cover's ink. */
  mark: { background: string; boxShadow: string; color: string };
}

export interface FitOptions {
  /** The wash is drawn inside a tile (the Latest card): the tile's white is
   *  under it, not over it. */
  onTile?: boolean;
  /** Told the composite checks the fit made (a test bounds them). */
  count?: (checks: number) => void;
}

/** A wash color's luminance against the surface's on a dark theme: a touch
 *  under it, so the light words on it only ever gain contrast; the deep shade
 *  at the foot of the page darker still. */
const DARK_WASH_LUMINANCE = 0.97;
const DARK_DEEP_LUMINANCE = 0.8;

/** The lightness at which this chroma and hue has luminance `y`. */
function lightnessAt(y: number, c: number, h: number): number {
  let lo = 0;
  let hi = 1;
  for (let i = 0; i < 24; i++) {
    const mid = (lo + hi) / 2;
    const rgb = fromLch(mid, c, h);
    if (luminance(rgb[0], rgb[1], rgb[2]) < y) lo = mid;
    else hi = mid;
  }
  return lo;
}

/**
 * The cover's colors toned for the surface: each keeps its hue, its chroma
 * lifted (`clamp(source * 2, floor, 0.15)`). On a light theme each sits a step
 * darker than the surface (`offset`). On a dark theme each sits at the
 * surface's own luminance, a touch under it: the theme's muted words already
 * sit under 4.5:1 on its raised surfaces (Rust floors them on the page
 * background), so any lighter wash would cost them contrast, and a wash that
 * keeps the light stays as dark as the theme is. When `rows` is set, the dark
 * theme uses the lighter offsets too: a row's hover lifts, and rowTint falls
 * back to the theme's own hover when that does not read.
 */
function toned(main: Lch, second: Lch | null, plain: Rgb, light: boolean, rows = false): Fit['colors'] {
  const surfaceL = lchOf(plain)[0];
  const surfaceY = luminance(plain[0], plain[1], plain[2]);
  const tone = (src: Lch, offset: number, floor: number, darkShare: number): Rgb => {
    const c = Math.min(0.15, Math.max(floor, src[1] * 2));
    const l = light || rows ? surfaceL + offset : lightnessAt(surfaceY * darkShare, c, src[2]);
    return fromLch(l, c, src[2]);
  };
  const secondSrc: Lch = second ?? [Math.min(1, Math.max(0, main[0] + (light ? -0.14 : 0.14))), main[1], main[2]];
  const deepSrc: Lch = [main[0] * 0.55, main[1] * 0.9, main[2]];
  return {
    main: tone(main, light ? -0.1 : 0.14, 0.05, DARK_WASH_LUMINANCE),
    second: tone(secondSrc, light ? -0.07 : 0.17, 0.045, DARK_WASH_LUMINANCE),
    deep: tone(deepSrc, light ? -0.04 : 0.05, 0.04, DARK_DEEP_LUMINANCE),
  };
}

/** The contrast muted text has over `c`. */
function mutedRatio(muted: Rgba, c: Rgb): number {
  const a = muted[3];
  const tr = c[0] + (muted[0] - c[0]) * a;
  const tg = c[1] + (muted[1] - c[1]) * a;
  const tb = c[2] + (muted[2] - c[2]) * a;
  return contrastOf(luminance(tr, tg, tb), luminance(c[0], c[1], c[2]));
}

/** The points checked on a w x h surface, as fractions, and each layer's
 *  falloff `max(0, 1 - d)` at them, kept per size. */
interface Geometry {
  xs: Float64Array;
  ys: Float64Array;
  /** Point-major: falloff[p * layers + l]. */
  falloff: Float64Array;
}

const geometries = new WeakMap<LayerDef[], Map<string, Geometry>>();
const KEPT_SIZES = 8;

function axis(len: number): number[] {
  const out: number[] = [];
  for (let v = 0; v < len; v += GRID_PX) out.push(v / len);
  out.push(1);
  return out;
}

function geometry(w: number, h: number, layers: LayerDef[]): Geometry {
  let bySize = geometries.get(layers);
  if (!bySize) {
    bySize = new Map();
    geometries.set(layers, bySize);
  }
  const key = `${w}x${h}`;
  const kept = bySize.get(key);
  if (kept) return kept;
  const xs: number[] = [];
  const ys: number[] = [];
  // A grid every 20 px with the edges and corners, and every layer's center
  // brought inside the box.
  const ax = axis(w);
  const ay = axis(h);
  for (const x of ax) {
    for (const y of ay) {
      xs.push(x);
      ys.push(y);
    }
  }
  for (const l of layers) {
    xs.push(Math.min(1, Math.max(0, l.cx)));
    ys.push(Math.min(1, Math.max(0, l.cy)));
  }
  const n = xs.length;
  const falloff = new Float64Array(n * layers.length);
  for (let p = 0; p < n; p++) {
    for (let li = 0; li < layers.length; li++) {
      const l = layers[li];
      const d = Math.hypot((xs[p] - l.cx) / l.rx, (ys[p] - l.cy) / l.ry);
      falloff[p * layers.length + li] = Math.max(0, 1 - d);
    }
  }
  const g = { xs: Float64Array.from(xs), ys: Float64Array.from(ys), falloff };
  if (bySize.size >= KEPT_SIZES) bySize.delete(bySize.keys().next().value as string);
  bySize.set(key, g);
  return g;
}

/** One base a point is checked on: the color under the wash, the white over
 *  it (a tile on the wash), and the contrast the muted words must keep. */
interface Base {
  under: Rgb;
  after: number;
  need: number;
}

function basesOf(s: Surface, onTile: boolean): Base[] {
  const plain = plainOf(s);
  const lit = over(WHITE, s.topLight, plain);
  const unders = s.topLight > 0 ? [plain, lit] : [plain];
  const out: Base[] = [];
  for (const u of unders) {
    for (const tile of s.tile > 0 ? [0, s.tile] : [0]) {
      const under = onTile ? over(WHITE, tile, u) : u;
      const after = onTile ? 0 : tile;
      const shown = after > 0 ? over(WHITE, after, under) : under;
      out.push({ under, after, need: Math.min(NEED_MAX, mutedRatio(s.muted, shown)) });
    }
  }
  return out;
}

/**
 * Fit a cover's colors to a w x h surface. Null for a gray cover or none, so
 * the surface stays exactly as the theme paints it.
 */
export function fitTint(
  tint: CoverTint | null | undefined,
  s: Surface,
  w: number,
  h: number,
  layers: LayerDef[] = PANEL_LAYERS,
  opts: FitOptions = {},
): Fit | null {
  const main = tint?.main;
  if (!main || !(w > 0) || !(h > 0)) return null;
  const onTile = !!opts.onTile;
  const colors = toned(main, tint.second ?? null, plainOf(s), s.light);
  const bases = basesOf(s, onTile);
  const g = geometry(Math.round(w), Math.round(h), layers);
  const n = g.xs.length;
  const nl = layers.length;
  const layerRgb = layers.map((l) => colors[l.color]);
  const mainRgb = colors.main;
  let checks = 0;

  // The wash at point p over `under`, at scale k (bottom up: the fill, then
  // the layers in reverse CSS order).
  const washAt = (p: number, under: Rgb, k: number): Rgb => {
    const fa = FILL_MOST * k;
    let r = under[0] + (mainRgb[0] - under[0]) * fa;
    let gg = under[1] + (mainRgb[1] - under[1]) * fa;
    let b = under[2] + (mainRgb[2] - under[2]) * fa;
    for (let li = nl - 1; li >= 0; li--) {
      const a = layers[li].most * k * g.falloff[p * nl + li];
      if (a <= 0) continue;
      const c = layerRgb[li];
      r += (c[0] - r) * a;
      gg += (c[1] - gg) * a;
      b += (c[2] - b) * a;
    }
    return [r, gg, b];
  };

  const reads = (k: number): boolean => {
    for (let p = 0; p < n; p++) {
      for (const base of bases) {
        checks++;
        let c = washAt(p, base.under, k);
        if (base.after > 0) c = over(WHITE, base.after, c);
        if (mutedRatio(s.muted, c) < base.need + MARGIN) return false;
      }
    }
    return true;
  };

  let k = 1;
  if (!reads(1)) {
    let lo = 0;
    let hi = 1;
    for (let i = 1; i < K_CHECKS; i++) {
      const mid = (lo + hi) / 2;
      if (reads(mid)) lo = mid;
      else hi = mid;
    }
    k = lo;
  }

  // Ink: the main hue, at the lightness that keeps 4.5:1 on the wash's worst
  // base (the lightest on a dark theme, the darkest on a light one).
  const worst = bases.reduce((a, b) => {
    const la = luminance(...over(WHITE, a.after, a.under));
    const lb = luminance(...over(WHITE, b.after, b.under));
    return s.light ? (lb < la ? b : a) : lb > la ? b : a;
  });
  const inkC = Math.min(0.16, Math.max(0.07, main[1] * 1.8));
  let inkL = s.light ? 0.52 : 0.8;
  let inkRgb = fromLch(inkL, inkC, main[2]);
  const inkReads = (): boolean => {
    const li = luminance(inkRgb[0], inkRgb[1], inkRgb[2]);
    for (let p = 0; p < n; p++) {
      checks++;
      let c = washAt(p, worst.under, k);
      if (worst.after > 0) c = over(WHITE, worst.after, c);
      if (contrastOf(li, luminance(c[0], c[1], c[2])) < INK_NEED) return false;
    }
    return true;
  };
  for (let i = 0; i < INK_CHECKS && !inkReads(); i++) {
    inkL += s.light ? -0.015 : 0.012;
    inkRgb = fromLch(inkL, inkC, main[2]);
  }
  opts.count?.(checks);

  const ink = rgbCss(inkRgb);
  return {
    k,
    colors,
    layers,
    fill: FILL_MOST,
    ink,
    inkRgb,
    glow: rgbaCss(colors.main, 0.7),
    mark: {
      background: `linear-gradient(150deg, ${rgbaCss(inkRgb, 0.3)}, ${rgbaCss(inkRgb, 0.1)})`,
      boxShadow: `inset 0 1px 0 rgba(255, 255, 255, 0.14), inset 0 0 0 1px ${rgbaCss(inkRgb, 0.26)}`,
      color: ink,
    },
  };
}

/**
 * The wash as a CSS `background` for a box inside the surface (x, y from the
 * surface's top left, in px): the same layers, placed in px relative to the
 * box, so the panel's own layer (box = the whole surface) and an edge fade
 * (box = its strip) paint the same color at the same point. CSS draws the
 * first layer on top: second, main, deep, then the flat fill.
 */
export function washBackground(
  fit: Fit,
  surface: { w: number; h: number },
  box: { x: number; y: number; w: number; h: number },
): string {
  const parts = fit.layers.map((l) => {
    const c = fit.colors[l.color];
    const a = l.most * fit.k;
    return (
      `radial-gradient(${num(l.rx * surface.w)}px ${num(l.ry * surface.h)}px at ` +
      `${num(l.cx * surface.w - box.x)}px ${num(l.cy * surface.h - box.y)}px, ` +
      `${rgbaCss(c, a)} 0%, ${rgbaCss(c, 0)} 100%)`
    );
  });
  const fill = rgbaCss(fit.colors.main, fit.fill * fit.k);
  parts.push(`linear-gradient(${fill}, ${fill})`);
  return parts.join(', ');
}

/**
 * A list row's hover and focus fill: the cover's toned main color at the
 * largest alpha (0.6 down in 0.02 steps) that keeps the muted words on the
 * plain surface, with and without the top light. Null keeps the row's
 * ordinary hover (`bg-glass-hover`): a gray cover, or no alpha that reads.
 */
export function rowTint(tint: CoverTint | null | undefined, s: Surface): string | null {
  const main = tint?.main;
  if (!main) return null;
  const plain = plainOf(s);
  const color = toned(main, tint.second ?? null, plain, s.light, true).main;
  const unders = s.topLight > 0 ? [plain, over(WHITE, s.topLight, plain)] : [plain];
  const needs = unders.map((u) => Math.min(NEED_MAX, mutedRatio(s.muted, u)));
  for (let step = 30; step >= 1; step--) {
    const a = step * 0.02;
    if (unders.every((u, i) => mutedRatio(s.muted, over(color, a, u)) >= needs[i] + MARGIN)) {
      return `rgba(${color[0]}, ${color[1]}, ${color[2]}, ${(step * 2) / 100})`;
    }
  }
  return null;
}

// ---------------------------------------------------------------- the hook

/** A size change waits this long to settle before the wash is fitted again;
 *  meanwhile the last fit's layers (fractions of the box) are drawn as they are. */
export const FIT_SETTLE_MS = 150;

export interface Size {
  w: number;
  h: number;
}

/** The size to fit for, given the size last fitted and the size now: the
 *  first real size at once, a change only once it has held (`wait`). */
export function fitSize(settled: Size, want: Size): { size: Size; wait: boolean } {
  if (!(settled.w > 0) || !(settled.h > 0)) return { size: want, wait: false };
  if (settled.w === want.w && settled.h === want.h) return { size: settled, wait: false };
  return { size: settled, wait: true };
}

/** A stable key for a tint's values (rows bring a new object on every page). */
export function tintKey(tint: CoverTint | null | undefined): string {
  return tint?.main ? JSON.stringify([tint.main, tint.second ?? null]) : '';
}

function tintOfKey(key: string): CoverTint | null {
  if (!key) return null;
  const [main, second] = JSON.parse(key) as [Lch, Lch | null];
  return { main, second };
}

const serverVersion = () => 0;

/**
 * The fit of a cover's colors for a surface of this kind and size, fitted
 * again when the theme changes (themeVersion) and once a resize has settled.
 * Null for a gray cover, a tint not here yet, or no page to read the theme
 * from (server markup).
 */
export function useCoverFit(
  tint: CoverTint | null | undefined,
  kind: SurfaceKind,
  width: number,
  height: number,
  layers: LayerDef[] = PANEL_LAYERS,
  onTile = false,
): Fit | null {
  const version = useSyncExternalStore(subscribeTheme, themeVersion, serverVersion);
  // A size not measured yet reads as none.
  const w = Number.isFinite(width) ? width : 0;
  const h = Number.isFinite(height) ? height : 0;
  const [settled, setSettled] = useState<Size>(() => ({ w, h }));
  const { size, wait } = fitSize(settled, { w, h });
  // The first real size is taken as it renders (React's way to keep a value
  // from the last render); a later one once it has held.
  if (!wait && (settled.w !== w || settled.h !== h)) setSettled({ w, h });
  useEffect(() => {
    if (!wait) return;
    const t = setTimeout(() => setSettled({ w, h }), FIT_SETTLE_MS);
    return () => clearTimeout(t);
  }, [w, h, wait]);
  const key = tintKey(tint);
  return useMemo(() => {
    if (typeof document === 'undefined') return null;
    // The version is read so a theme change refits.
    void version;
    return fitTint(tintOfKey(key), readSurface(kind), size.w, size.h, layers, { onTile });
  }, [key, version, kind, size.w, size.h, layers, onTile]);
}

// ---------------------------------------------------------------- for tests

/** One layer of a CSS background as emitted here. */
export type ParsedLayer =
  | {
      kind: 'radial';
      /** Radii and center, in `unit` (px from the box's top left, or % of it). */
      rx: number;
      ry: number;
      cx: number;
      cy: number;
      unit: 'px' | '%';
      rgb: Rgb;
      /** Alpha at the center and at the edge of the ellipse. */
      alpha: number;
      alphaEnd: number;
    }
  | { kind: 'fill'; rgb: Rgb; alpha: number }
  | { kind: 'other'; text: string };

/** Split at commas outside parentheses. */
function topLevel(s: string): string[] {
  const out: string[] = [];
  let depth = 0;
  let start = 0;
  for (let i = 0; i < s.length; i++) {
    const ch = s[i];
    if (ch === '(') depth++;
    else if (ch === ')') depth--;
    else if (ch === ',' && depth === 0) {
      out.push(s.slice(start, i).trim());
      start = i + 1;
    }
  }
  out.push(s.slice(start).trim());
  return out.filter(Boolean);
}

function stop(text: string): { rgb: Rgb; alpha: number } | null {
  const m = /^(rgba?\([^)]*\)|#[0-9a-f]+)/i.exec(text.trim());
  const c = m ? parseColor(m[1]) : null;
  return c ? { rgb: [c[0], c[1], c[2]], alpha: c[3] } : null;
}

/** Read a background back into its layers, top first (for tests). */
export function parseBackground(css: string): ParsedLayer[] {
  return topLevel(css).map((part): ParsedLayer => {
    const radial = /^radial-gradient\((.*)\)$/i.exec(part);
    if (radial) {
      const [shape, from, to] = topLevel(radial[1]);
      const m = /^(-?[\d.]+)(px|%)\s+(-?[\d.]+)(?:px|%)\s+at\s+(-?[\d.]+)(?:px|%)\s+(-?[\d.]+)(?:px|%)$/i.exec(shape ?? '');
      const a = from ? stop(from) : null;
      const b = to ? stop(to) : null;
      if (m && a && b) {
        return {
          kind: 'radial',
          rx: parseFloat(m[1]),
          ry: parseFloat(m[3]),
          cx: parseFloat(m[4]),
          cy: parseFloat(m[5]),
          unit: m[2] === '%' ? '%' : 'px',
          rgb: a.rgb,
          alpha: a.alpha,
          alphaEnd: b.alpha,
        };
      }
    }
    const linear = /^linear-gradient\((.*)\)$/i.exec(part);
    if (linear) {
      const stops = topLevel(linear[1]).map(stop);
      if (stops.length === 2 && stops[0] && stops[1] && stops[0].alpha === stops[1].alpha) {
        return { kind: 'fill', rgb: stops[0].rgb, alpha: stops[0].alpha };
      }
    }
    return { kind: 'other', text: part };
  });
}
