import { useEffect, useMemo, useState, type CSSProperties } from 'react';

import type { ModRequest } from '../../services/moderation';
import {
  LINE_GEOMETRY,
  LINE_STRIP_ATTR,
  lineHit,
  lineLayout,
  lineRequest,
  type LineButton,
  type LineLayout,
} from './modLine';

// The line buttons' picture, and reading the pointer back from it.
//
// Every strip is one empty element showing the same picture: the buttons at
// rest on its first row, then one row per button with that button lit. The
// picture is drawn once per list (buttons, text size, theme, font, screen
// density) on a canvas, in the app's font and the theme's colors, so it looks
// like the CSS buttons it replaced. Hover moves the hovered strip's picture to
// its lit row; a click picks the button under the pointer.

export interface LineArt {
  layout: LineLayout;
  /** Rows in the picture: rest, then one per button. */
  rows: number;
  /** The variables the strips draw with; set on the list. */
  vars: CSSProperties;
}

// Lucide's trash-2 and ban, in their 24 px box.
const ICONS: Record<'delete' | 'ban', string[]> = {
  delete: [
    'M3 6h18',
    'M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6',
    'M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2',
    'M10 11v6',
    'M14 11v6',
  ],
  ban: ['M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0', 'm4.9 4.9 14.2 14.2'],
};

/** Theme colors the recipes below read, as :root has them now. */
function rootVars(): (css: string) => string {
  const root = getComputedStyle(document.documentElement);
  return (css) => css.replace(/var\((--[\w-]+)\)/g, (_, name: string) => root.getPropertyValue(name).trim() || 'transparent');
}

/** `css` as a canvas color, or `fallback` when the canvas cannot read it. */
function canvasColor(ctx: CanvasRenderingContext2D, css: string, fallback: string): string {
  ctx.fillStyle = '#010203';
  ctx.fillStyle = css;
  const parsed = String(ctx.fillStyle);
  return parsed === '#010203' ? fallback : parsed;
}

function drawLineArt(buttons: LineButton[], fontPx: number): LineArt | null {
  if (buttons.length === 0 || typeof document === 'undefined') return null;
  const canvas = document.createElement('canvas');
  const ctx = canvas.getContext('2d');
  if (!ctx) return null;
  const g = LINE_GEOMETRY;
  const family = getComputedStyle(document.body).fontFamily || 'sans-serif';
  // 600, not bold: a bold label outweighed the icons beside it.
  const labelFont = `600 ${g.label * fontPx}px ${family}`;
  ctx.font = labelFont;
  const layout = lineLayout(buttons, fontPx, (label) => ctx.measureText(label).width);
  const rows = buttons.length + 1;
  // Whole-number density, so every edge lands on a device pixel at 1x and 2x.
  const scale = Math.max(1, Math.ceil(window.devicePixelRatio || 1));
  canvas.width = Math.max(1, Math.ceil(layout.width * scale));
  canvas.height = Math.max(1, layout.height * rows * scale);

  // A faint wash at rest, the step's color mixed into the ink; the step's
  // color as the fill when lit, with ink picked from it (near black on light
  // fills, white on dark). The rest ink mixes with a NEUTRAL gray at the
  // theme's secondary-text lightness: the text color itself leans blue, which
  // turned the amber timeouts olive.
  const resolve = rootVars();
  const rest = canvasColor(ctx, resolve('color-mix(in srgb, var(--color-text-primary) 7%, transparent)'), 'rgba(255,255,255,0.07)');
  const neutral = canvasColor(ctx, resolve('oklch(from var(--color-text-secondary) l 0 0)'), '#969696');
  const paints = buttons.map((b) => {
    const tint = resolve(b.tint);
    // Delete's slate is darker than the text, so half of it smudged the
    // trash into the wash; a quarter keeps it as legible as the Ban icon.
    const restShare = b.kind === 'delete' ? 25 : 50;
    // Lit, Delete's own slate barely stood off the row; a light slate does.
    const lit = b.kind === 'delete' ? resolve('color-mix(in srgb, var(--color-text-secondary) 50%, #94a3b8)') : tint;
    return {
      restInk: canvasColor(ctx, `color-mix(in srgb, ${tint} ${restShare}%, ${neutral})`, '#9ca3af'),
      litFill: canvasColor(ctx, `color-mix(in srgb, ${lit} 85%, transparent)`, lit),
      litInk: canvasColor(ctx, `oklch(from ${lit} clamp(0.15, (0.655 - l) * 1000, 1) 0 0)`, '#ffffff'),
    };
  });

  ctx.scale(scale, scale);
  ctx.lineCap = 'round';
  ctx.lineJoin = 'round';
  ctx.textAlign = 'center';
  // Center the label's ink, not its em box: digits and "m" have no
  // descenders, so the em box's middle sat the ink a pixel low.
  ctx.textBaseline = 'alphabetic';
  ctx.font = labelFont;
  // Icons at a whole pixel size with Lucide's 2/24 stroke rounded to whole
  // device pixels and laid on the pixel grid, so they stay sharp at 1x (an
  // 11.2 px icon with a 0.93 px stroke smeared). The trash drops its inner
  // bars when it is too small to keep them apart.
  const iconPx = Math.max(12, Math.round(g.icon * fontPx));
  const deviceStroke = Math.max(1, Math.round((iconPx * scale) / 12));
  const strokeOffset = deviceStroke % 2 === 1 ? 0.5 / scale : 0;
  const snap = (v: number) => Math.round(v * scale) / scale + strokeOffset;
  for (let row = 0; row < rows; row++) {
    const top = row * layout.height;
    buttons.forEach((b, i) => {
      const box = layout.boxes[i];
      const lit = row === i + 1;
      const paint = paints[i];
      ctx.fillStyle = lit ? paint.litFill : rest;
      ctx.beginPath();
      ctx.roundRect(box.x, top, box.w, layout.height, g.radius * fontPx);
      ctx.fill();
      const ink = lit ? paint.litInk : paint.restInk;
      if (b.kind === 'timeout') {
        const m = ctx.measureText(b.label);
        ctx.fillStyle = ink;
        ctx.fillText(
          b.label,
          box.x + box.w / 2,
          top + layout.height / 2 + (m.actualBoundingBoxAscent - m.actualBoundingBoxDescent) / 2,
        );
        return;
      }
      ctx.save();
      ctx.translate(snap(box.x + (box.w - iconPx) / 2), snap(top + (layout.height - iconPx) / 2));
      ctx.scale(iconPx / 24, iconPx / 24);
      ctx.strokeStyle = ink;
      ctx.lineWidth = ((deviceStroke / scale) * 24) / iconPx;
      const paths = b.kind === 'delete' && iconPx * scale < 14 ? ICONS.delete.slice(0, 3) : ICONS[b.kind];
      for (const d of paths) ctx.stroke(new Path2D(d));
      ctx.restore();
    });
  }

  return {
    layout,
    rows,
    vars: {
      '--sn-line-art': `url(${canvas.toDataURL('image/png')})`,
      '--sn-line-w': `${layout.width}px`,
      '--sn-line-h': `${layout.height}px`,
      '--sn-line-bh': `${layout.height * rows}px`,
    } as CSSProperties,
  };
}

/**
 * The picture for `buttons` at `fontPx`, redrawn when the theme or the app
 * font changes (both are written onto :root), when a font finishes loading,
 * and when the window moves to a screen of another density. Null while off.
 */
export function useLineArt(buttons: LineButton[] | null, fontPx: number): LineArt | null {
  const [epoch, setEpoch] = useState(0);
  const active = buttons !== null;
  useEffect(() => {
    if (!active) return;
    const redraw = () => setEpoch((n) => n + 1);
    const themeWatch = new MutationObserver(redraw);
    themeWatch.observe(document.documentElement, { attributes: true, attributeFilter: ['style', 'class', 'data-theme'] });
    document.fonts?.addEventListener('loadingdone', redraw);
    const density = window.matchMedia(`(resolution: ${window.devicePixelRatio || 1}dppx)`);
    density.addEventListener('change', redraw);
    return () => {
      themeWatch.disconnect();
      document.fonts?.removeEventListener('loadingdone', redraw);
      density.removeEventListener('change', redraw);
    };
  }, [active]);
  // `epoch` stands for what the drawing reads from the page.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  return useMemo(() => (buttons ? drawLineArt(buttons, fontPx) : null), [buttons, fontPx, epoch]);
}

/** The strip a pointer event landed on, or null. */
function stripAt(target: EventTarget | null): HTMLElement | null {
  return target instanceof HTMLElement && target.hasAttribute(LINE_STRIP_ATTR) ? target : null;
}

/** Whether a pointer event landed on a strip. */
export function isOnLineStrip(target: EventTarget | null): boolean {
  return stripAt(target) !== null;
}

/** The button of `strip` under a pointer at `clientX`, or -1. */
function buttonUnder(strip: HTMLElement, clientX: number, art: LineArt): number {
  const rect = strip.getBoundingClientRect();
  // The strip is drawn at layout.width px; scale in case the page is zoomed.
  const x = ((clientX - rect.left) / (rect.width || 1)) * art.layout.width;
  return lineHit(art.layout, x);
}

/** Show `strip`'s lit row for button `index`, or its rest row for -1. */
function light(strip: HTMLElement, index: number, art: LineArt | null): void {
  if (index < 0 || !art) strip.style.removeProperty('background-position');
  else strip.style.backgroundPosition = `0 ${-(index + 1) * art.layout.height}px`;
}

/**
 * Hover and click for every strip under `el`, from one set of listeners:
 * the button under the pointer lights up, and a click (pressed and released
 * on the same button) hands its request to `onPick`. A press keeps the focus
 * where it was (the composer) and starts no text selection. Returns the
 * cleanup.
 */
export function bindLineStrips(
  el: HTMLElement,
  getArt: () => { art: LineArt; buttons: LineButton[] } | null,
  onPick: (hit: { request: ModRequest; name: string; label: string }) => void,
): () => void {
  let lit: HTMLElement | null = null;
  const unlight = () => {
    if (lit) light(lit, -1, null);
    lit = null;
  };
  const onMove = (e: PointerEvent) => {
    const strip = stripAt(e.target);
    const current = getArt();
    if (!strip || !current) {
      unlight();
      return;
    }
    if (lit && lit !== strip) unlight();
    lit = strip;
    light(strip, buttonUnder(strip, e.clientX, current.art), current.art);
  };
  const onOut = (e: PointerEvent) => {
    if (stripAt(e.target) && stripAt(e.target) !== stripAt(e.relatedTarget)) unlight();
  };
  // The button a press started on. The strip is one element, so the browser
  // would also count a press on 10m released over Ban as a click; like real
  // buttons, only a press and release on the same button act.
  let pressed: { strip: HTMLElement; index: number } | null = null;
  const onDown = (e: PointerEvent) => {
    const strip = stripAt(e.target);
    const current = getArt();
    pressed = strip && current && e.button === 0 ? { strip, index: buttonUnder(strip, e.clientX, current.art) } : null;
  };
  const onMouseDown = (e: MouseEvent) => {
    if (stripAt(e.target)) e.preventDefault();
  };
  const onClick = (e: MouseEvent) => {
    const strip = stripAt(e.target);
    const current = getArt();
    const start = pressed;
    pressed = null;
    if (!strip || !current || !start || start.strip !== strip) return;
    const index = buttonUnder(strip, e.clientX, current.art);
    if (index < 0 || index !== start.index) return;
    const hit = lineRequest(current.buttons, index, strip.dataset);
    if (hit) onPick(hit);
  };
  el.addEventListener('pointermove', onMove);
  el.addEventListener('pointerout', onOut);
  el.addEventListener('pointerdown', onDown);
  el.addEventListener('mousedown', onMouseDown);
  el.addEventListener('click', onClick);
  return () => {
    unlight();
    el.removeEventListener('pointermove', onMove);
    el.removeEventListener('pointerout', onOut);
    el.removeEventListener('pointerdown', onDown);
    el.removeEventListener('mousedown', onMouseDown);
    el.removeEventListener('click', onClick);
  };
}
