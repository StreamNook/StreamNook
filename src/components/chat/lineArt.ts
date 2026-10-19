import { useCallback, useEffect, useMemo, useState, type CSSProperties } from 'react';

import type { ModRequest, ModStep } from '../../services/moderation';
import {
  LINE_GEOMETRY,
  LINE_STRIP_ATTR,
  lineButtons,
  lineHit,
  lineLayout,
  lineRequest,
  nextLength,
  scrolledTimeout,
  withLengths,
  type LineButton,
  type LineLayout,
} from './modLine';

// The line buttons' picture, and reading the pointer back from it.
//
// Every strip is one empty element showing the same picture: the buttons at
// rest as set on its first row, then at rest with any scrolled length (the
// line under the pointer), then one row per button with that button lit. The
// picture is drawn once per list (buttons, text size, theme, font, screen
// density) on a canvas, in the app's font and the theme's colors, so it looks
// like the CSS buttons it replaced. Hover moves the hovered strip's picture to
// its lit row; a click picks the button under the pointer.

export interface LineArt {
  layout: LineLayout;
  /** Rows in the picture: rest as set, rest as scrolled, then one per
   *  button. */
  rows: number;
  /** The picture itself, as a data URL. */
  url: string;
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

function drawLineArt(buttons: LineButton[], fontPx: number, fitLabels: string[], setButtons: LineButton[]): LineArt | null {
  if (buttons.length === 0 || typeof document === 'undefined') return null;
  const canvas = document.createElement('canvas');
  const ctx = canvas.getContext('2d');
  if (!ctx) return null;
  const g = LINE_GEOMETRY;
  const family = getComputedStyle(document.body).fontFamily || 'sans-serif';
  // 600, not bold: a bold label outweighed the icons beside it.
  const labelFont = `600 ${g.label * fontPx}px ${family}`;
  ctx.font = labelFont;
  const layout = lineLayout(buttons, fontPx, (label) => ctx.measureText(label).width, fitLabels);
  const rows = buttons.length + 2;
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
    // The first row is every other line: the lengths as set, never a length
    // scrolled on the line under the pointer.
    (row === 0 && setButtons.length === buttons.length ? setButtons : buttons).forEach((b, i) => {
      const box = layout.boxes[i];
      const lit = row === i + 2;
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

  const url = canvas.toDataURL('image/png');
  return {
    layout,
    rows,
    url,
    vars: {
      '--sn-line-art': `url(${url})`,
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
 * Timeouts are as wide as the widest of `fitLabels` (the lengths a scroll can
 * reach), so a scroll never resizes the strip.
 */
export function useLineArt(
  buttons: LineButton[] | null,
  fontPx: number,
  fitLabels: string[] = [],
  setButtons: LineButton[] | null = null,
): LineArt | null {
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
  const fitKey = fitLabels.join('|');
  // `epoch` stands for what the drawing reads from the page; `fitKey` for
  // the labels.
  const drawn = useMemo(
    () => (buttons ? drawLineArt(buttons, fontPx, fitLabels, setButtons ?? buttons) : null),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [buttons, fontPx, epoch, fitKey, setButtons],
  );
  // A new picture is decoded before it replaces the old one: swapped in
  // undecoded, every strip went blank for a frame on each scroll step.
  const [shown, setShown] = useState<LineArt | null>(null);
  useEffect(() => {
    if (!drawn) return;
    let live = true;
    const img = new Image();
    img.src = drawn.url;
    img
      .decode()
      .catch(() => undefined)
      .then(() => {
        if (live) setShown(drawn);
      });
    return () => {
      live = false;
    };
  }, [drawn]);
  return drawn ? shown : null;
}

/**
 * The line's buttons from Rust's steps, with any timeout whose length was
 * scrolled. `ladder` is the slider's timeouts (its Steps, as Rust applies
 * them): a scroll walks a timeout up or down that list. `nudge` takes the
 * button under the pointer and a direction and says whether the line has a
 * timeout to scroll (a null direction only asks); `reset` puts every
 * length back.
 */
export function useLineButtons(steps: ModStep[] | null, ladder: ModStep[]) {
  const [overrides, setOverrides] = useState<Record<number, number>>({});
  const buttons = useMemo(
    () => (steps && steps.length > 0 ? lineButtons(withLengths(steps, overrides, ladder)) : null),
    [steps, overrides, ladder],
  );
  const nudge = useCallback(
    (index: number, dir: 1 | -1 | null): boolean => {
      if (!steps) return false;
      const t = scrolledTimeout(steps, index);
      if (t < 0) return false;
      if (dir === null) return true;
      setOverrides((o) => {
        const current = o[t] ?? steps[t].seconds ?? 0;
        const next = nextLength(
          current,
          ladder.map((s) => s.seconds ?? 0).filter((s) => s > 0),
          dir,
        );
        return next == null ? o : { ...o, [t]: next };
      });
      return true;
    },
    [steps, ladder],
  );
  const reset = useCallback(() => setOverrides((o) => (Object.keys(o).length > 0 ? {} : o)), []);
  // What a scroll would do from here: the timeout it changes, and the
  // lengths one step up and one step down (null at either end).
  const around = useCallback(
    (index: number): ScrollHint | null => {
      if (!steps) return null;
      const t = scrolledTimeout(steps, index);
      if (t < 0) return null;
      const current = overrides[t] ?? steps[t].seconds ?? 0;
      const seconds = ladder.map((s) => s.seconds ?? 0).filter((s) => s > 0);
      const labelOf = (s: number | null) => (s == null ? null : ladder.find((l) => l.seconds === s)?.label ?? null);
      return { timeout: t, up: labelOf(nextLength(current, seconds, 1)), down: labelOf(nextLength(current, seconds, -1)) };
    },
    [steps, ladder, overrides],
  );
  return { buttons, nudge, reset, around };
}

/** The timeout a scroll changes and the lengths one step either way. */
export interface ScrollHint {
  timeout: number;
  up: string | null;
  down: string | null;
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
  if (index >= 0 && art) strip.style.backgroundPosition = `0 ${-(index + 2) * art.layout.height}px`;
  // The line whose length was scrolled keeps showing that length.
  else if (strip.dataset.scrolled) strip.style.backgroundPosition = `0 ${-strip.offsetHeight}px`;
  else strip.style.removeProperty('background-position');
}

/** A wheel event this big is one notch of a mouse wheel, one step whatever
 *  the display scaling (Chromium reports 100 per notch at 100%, 50 at 200%).
 *  Smaller ones are a trackpad, added up until they reach WHEEL_STEP. */
const WHEEL_NOTCH = 40;
const WHEEL_STEP = 100;

const CARET_UP =
  '<svg width="8" height="8" viewBox="0 0 8 8" aria-hidden="true"><path d="M1.5 5.5 4 3l2.5 2.5" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>';
const CARET_DOWN =
  '<svg width="8" height="8" viewBox="0 0 8 8" aria-hidden="true"><path d="M1.5 2.5 4 5l2.5-2.5" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>';

/**
 * Scrolling over a strip walks its timeout's length: up for longer, down for
 * shorter (`onNudge`, with the button under the pointer). The wheel is then
 * the strip's alone (chat does not scroll and keeps holding still). Leaving
 * the line puts the lengths back (`onReset`).
 *
 * While the pointer is on a strip, two small notes sit just above and below
 * its timeout: the length a scroll up reaches and the one a scroll down
 * reaches (`around`), like the next rungs of a picker wheel, so the scroll
 * is found without reading anything. Labels only, never buttons. One pair
 * for the whole page, fixed to the screen. Returns the cleanup.
 */
export function bindLineWheel(
  el: HTMLElement,
  getArt: () => { art: LineArt; buttons: LineButton[] } | null,
  onNudge: (index: number, dir: 1 | -1 | null) => boolean,
  onReset: () => void,
  around: (index: number) => ScrollHint | null,
): () => void {
  const hint = document.createElement('div');
  hint.className = 'sn-line-hint';
  hint.setAttribute('aria-hidden', 'true');
  const upNote = document.createElement('span');
  upNote.className = 'sn-line-hint-note is-up';
  const downNote = document.createElement('span');
  downNote.className = 'sn-line-hint-note is-down';
  hint.append(upNote, downNote);
  document.body.appendChild(hint);
  let aim: { strip: HTMLElement; index: number } | null = null;
  const hideHint = () => {
    aim = null;
    hint.classList.remove('is-on');
  };
  const showHint = () => {
    const current = getArt();
    const a = aim ? around(aim.index) : null;
    if (!aim || !current || !a || (!a.up && !a.down) || !aim.strip.isConnected) {
      hint.classList.remove('is-on');
      return;
    }
    const rect = aim.strip.getBoundingClientRect();
    const box = current.art.layout.boxes[a.timeout];
    if (!box || rect.width === 0) {
      hint.classList.remove('is-on');
      return;
    }
    const scale = rect.width / (current.art.layout.width || 1);
    const cx = rect.left + (box.x + box.w / 2) * scale;
    upNote.innerHTML = a.up ? `${CARET_UP}<span>${a.up}</span>` : '';
    downNote.innerHTML = a.down ? `${CARET_DOWN}<span>${a.down}</span>` : '';
    upNote.hidden = !a.up;
    downNote.hidden = !a.down;
    upNote.style.transform = `translate(${cx}px, ${rect.top - 2}px) translate(-50%, -100%)`;
    downNote.style.transform = `translate(${cx}px, ${rect.bottom + 2}px) translateX(-50%)`;
    hint.classList.add('is-on');
  };
  // After a scroll the page draws the new length first; read it after that.
  const showHintSoon = () => requestAnimationFrame(() => requestAnimationFrame(showHint));
  const onMove = (e: PointerEvent) => {
    const strip = stripAt(e.target);
    const current = getArt();
    if (!strip || !current || e.pointerType === 'touch') {
      if (aim) hideHint();
      return;
    }
    const index = buttonUnder(strip, e.clientX, current.art);
    if (aim && aim.strip === strip && aim.index === index) return;
    aim = { strip, index };
    showHint();
  };
  const onOut = (e: PointerEvent) => {
    if (aim && stripAt(e.target) && stripAt(e.target) !== stripAt(e.relatedTarget)) hideHint();
  };
  // Acting (a click) or the chat moving takes the notes away.
  const onDownHide = () => hideHint();
  const onScrollHide = () => hideHint();
  let acc = 0;
  let accAt = 0;
  let scrolledRow: Element | null = null;
  let scrolledStrip: HTMLElement | null = null;
  const clear = () => {
    if (scrolledStrip) {
      delete scrolledStrip.dataset.scrolled;
      scrolledStrip.style.removeProperty('background-position');
    }
    scrolledStrip = null;
    scrolledRow = null;
    onReset();
  };
  const rowOf = (strip: HTMLElement) => strip.closest('.chat-message-row') ?? strip.parentElement;
  const onWheel = (e: WheelEvent) => {
    const strip = stripAt(e.target);
    const current = getArt();
    if (!strip || !current || e.deltaY === 0) return;
    const index = buttonUnder(strip, e.clientX, current.art);
    let handled = false;
    if (e.deltaMode !== 0 || Math.abs(e.deltaY) >= WHEEL_NOTCH) {
      // A mouse notch: one step.
      acc = 0;
      handled = onNudge(index, e.deltaY < 0 ? 1 : -1);
    } else {
      // A trackpad: small deltas add up.
      if (e.timeStamp - accAt > 250) acc = 0;
      accAt = e.timeStamp;
      acc += e.deltaY;
      while (Math.abs(acc) >= WHEEL_STEP) {
        const dir: 1 | -1 = acc < 0 ? 1 : -1;
        acc += dir * WHEEL_STEP;
        handled = onNudge(index, dir) || handled;
      }
    }
    // Mid-gesture (below a whole step) the strip still owns the wheel, as
    // long as the line has a timeout to scroll.
    if (!handled && !onNudge(index, null)) return;
    e.preventDefault();
    e.stopPropagation();
    if (handled) {
      if (scrolledStrip && scrolledStrip !== strip) clear();
      scrolledStrip = strip;
      strip.dataset.scrolled = 'true';
      scrolledRow = rowOf(strip);
      aim = { strip, index };
      showHintSoon();
    }
  };
  const onOver = (e: PointerEvent) => {
    if (!scrolledRow || (e.target instanceof Node && scrolledRow.contains(e.target))) return;
    clear();
  };
  const onLeave = () => {
    if (scrolledRow) clear();
  };
  el.addEventListener('wheel', onWheel, { passive: false });
  el.addEventListener('pointerover', onOver);
  el.addEventListener('pointerleave', onLeave);
  el.addEventListener('pointermove', onMove);
  el.addEventListener('pointerout', onOut);
  el.addEventListener('pointerdown', onDownHide);
  el.addEventListener('scroll', onScrollHide, { passive: true });
  return () => {
    el.removeEventListener('wheel', onWheel);
    el.removeEventListener('pointerover', onOver);
    el.removeEventListener('pointerleave', onLeave);
    el.removeEventListener('pointermove', onMove);
    el.removeEventListener('pointerout', onOut);
    el.removeEventListener('pointerdown', onDownHide);
    el.removeEventListener('scroll', onScrollHide);
    hint.remove();
  };
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
