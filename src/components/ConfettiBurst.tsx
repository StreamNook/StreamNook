import { useEffect, useRef } from 'react';

/**
 * Confetti celebration for Hype Train level-ups (and the offline room's
 * corner hits).
 *
 * Renders a canvas over its host (the nearest parent): a burst of paper
 * confetti erupts upward near the top and then flutters down. The paper
 * "flip" is faked by oscillating each piece's vertical scale with a cosine
 * and swapping a front/back color, and the lazy side-to-side sway comes from
 * drag plus a per-frame random nudge.
 *
 * With `perchOn`, the direct children of the matching row (the chat
 * composer's wallet, tools, field and Send) catch some of it: a piece that
 * reaches one of their top edges settles there, lying flat along the edge
 * (round corners included) and piling a little on what landed before. It
 * stays until the pointer brushes past it (a mouse hovering, a finger or pen
 * dragging): pieces within BRUSH_RADIUS of it flick off the way it is moving,
 * and what lay on them slides off too, so a sweep follows its path. A tap, a
 * click or keyboard focus clears the whole control. A control
 * that goes away drops what sat on it.
 *
 * One instance lives as long as anything is falling or resting. Bump `burst`
 * to throw another burst into the same canvas without losing what landed;
 * `onDone` fires once nothing is left, so the owner can unmount it. The frame
 * loop runs only while something moves: resting confetti is drawn once and
 * redrawn when the host or a perch resizes.
 */

interface ColorPair {
  front: string;
  back: string; // darker shade shown while a piece is flipped away from us
}

// Echoes the rainbow hype-train gradient.
const RAINBOW: ColorPair[] = [
  { front: '#ff5b9e', back: '#d63d7d' }, // pink
  { front: '#9b6bff', back: '#7a4fe0' }, // purple
  { front: '#4d96ff', back: '#345dd1' }, // blue
  { front: '#3ed8e0', back: '#2bb0b8' }, // cyan
  { front: '#ffd93d', back: '#e0b81f' }, // yellow
  { front: '#6bcb77', back: '#4ea85a' }, // green
];

// Used for golden-kappa hype trains.
const GOLD: ColorPair[] = [
  { front: '#FFD700', back: '#D4AF37' },
  { front: '#FFF1B8', back: '#E6C200' },
  { front: '#FFB300', back: '#CC8F00' },
];

// "Physics" tuning for confetti and sequin fall.
const GRAVITY_CONFETTI = 0.3;
const GRAVITY_SEQUINS = 0.55;
const DRAG_CONFETTI = 0.075;
const DRAG_SEQUINS = 0.02;
const TERMINAL_VELOCITY = 3;

const CONFETTI_COUNT = 60;
const SEQUIN_COUNT = 24;

// Landing. Only some pieces land, so the controls get a sprinkle rather than
// a carpet; the rest fall past in front of them.
const LAND_CHANCE_CONFETTI = 0.55;
const LAND_CHANCE_SEQUINS = 0.4;
const MAX_RESTING = 80;
const MAX_TIP = 0.75; // rad a piece tips, held up at one end, before it stops
const PILE_BIN = 3; // px per pile column
const MAX_PILE = 6; // px a pile may reach before pieces fall past it
const MAX_CORNER_DROP = 5; // px below the flat top a rounded corner still catches
const SETTLE_FRAMES = 14;
const SETTLE_BOUNCE = 3;
// Sweeping. The brush reaches pieces one frame per this many px from the press.
const SWEEP_PX_PER_FRAME = 9;
const SWEEP_MAX_DELAY = 20;
const SWEEP_GROW_FRAMES = 8;
// How far above a control a tap still counts as touching its confetti.
const REACH_ABOVE = 14;
// How close the hovering pointer comes before it flicks a piece off.
const BRUSH_RADIUS = 16;
// How far a finger may move and still count as a tap.
const TAP_SLOP = 6;
// Remeasure the perches every this many frames while anything moves.
const REMEASURE_EVERY = 12;

const rand = (min: number, max: number) => Math.random() * (max - min) + min;
const pick = <T,>(arr: T[]) => arr[Math.floor(Math.random() * arr.length)];
const lerp = (a: number, b: number, t: number) => a + (b - a) * t;

// Weighted upward spread so most pieces launch near the top of the range,
// which reads as a livelier burst than a flat random.
function initConfettoVelocity(xRange: [number, number], yRange: [number, number]) {
  const x = rand(xRange[0], xRange[1]);
  const range = yRange[1] - yRange[0] + 1;
  let y = yRange[1] - Math.abs(rand(0, range) + rand(0, range) - range);
  if (y >= yRange[1] - 1) {
    y += Math.random() < 0.25 ? rand(1, 3) : 0;
  }
  return { x, y: -y }; // negative y is upward
}

/** A control confetti can land on, in host coordinates. */
interface Perch {
  el: Element;
  /** Lands on the control's icon (a bare icon button) rather than its box.
   *  Decided once, so a hover background never moves the edge under it. */
  glyph: boolean;
  /** The row child draws its own box and is the surface (else its control
   *  inside is). Decided once, like `glyph`. */
  self: boolean;
  /** The whole control, where a press sweeps it. */
  hit: { left: number; top: number; right: number; bottom: number };
  left: number;
  top: number;
  width: number;
  height: number;
  radius: number;
  /** The bare edge per PILE_BIN-wide column, as a height above the perch's
   *  top: below 0 on a rounded corner, or an icon's drawn outline once it is
   *  read (`outline`). NO_GROUND where nothing can hold a piece. */
  base: Float32Array;
  /** An icon perch's outline, read from its pixels; null until read. */
  outline: Float32Array | null;
  /** What a falling piece meets now: `base`, raised by the confetti lying on it. */
  ground: Float32Array;
}

const NO_GROUND = -1e6;

// fall: in the air. settle: just landed, easing flat. rest: lying on a perch.
// swept: brushed, waiting `t` frames for the brush to reach it, then airborne.
type Mode = 'fall' | 'settle' | 'rest' | 'swept';

interface Piece {
  sequin: boolean;
  color: ColorPair;
  /** Paper: width and height. Sequin: radius in `w`. */
  w: number;
  h: number;
  x: number;
  y: number;
  vx: number;
  vy: number;
  rot: number;
  flip: number; // phase offset for the paper flip
  scaleY: number;
  lands: boolean;
  mode: Mode;
  t: number;
  // Resting shape and place, relative to the perch: `offX` from its left,
  // `lift` above its top (to the piece's center), lying along `restRot`.
  perch: Perch | null;
  offX: number;
  lift: number;
  restRot: number;
  restLen: number;
  restThick: number;
  // Shape at the moment of landing, eased from while settling.
  fromW: number;
  fromH: number;
  fromRot: number;
  // Frames left growing back from flat to paper after a sweep.
  grow: number;
}

/** The top edge's y (relative to the perch top) at local x, following the
 *  rounded corners; null where the corner has curved away too far to hold. */
function surfaceAt(p: Perch, lx: number): number | null {
  if (lx < 0 || lx > p.width) return null;
  const r = p.radius;
  const d = lx < r ? r - lx : lx > p.width - r ? lx - (p.width - r) : 0;
  if (d === 0) return 0;
  const drop = r - Math.sqrt(Math.max(0, r * r - d * d));
  return drop > MAX_CORNER_DROP ? null : drop;
}

/** A computed color's alpha: rgba(), "rgb(... / a)", color(... / a). */
function alphaOf(color: string): number {
  if (!color || color === 'transparent') return 0;
  const slash = /\/\s*([\d.]+)(%?)\s*\)\s*$/.exec(color);
  if (slash) return parseFloat(slash[1]) / (slash[2] ? 100 : 1);
  const rgba = /^rgba\(([^)]+)\)$/.exec(color);
  if (rgba) {
    const parts = rgba[1].split(',');
    if (parts.length === 4) return parseFloat(parts[3]);
  }
  return 1;
}

/** The control draws a box you can see: a fill, an image or a border, on
 *  itself or on its `::before` (where the glaze paints its material). */
function drawsBox(el: Element): boolean {
  const paints = (cs: CSSStyleDeclaration) =>
    alphaOf(cs.backgroundColor) > 0.04 ||
    (cs.backgroundImage !== 'none' && cs.backgroundImage !== '') ||
    (parseFloat(cs.borderTopWidth) > 0 && alphaOf(cs.borderTopColor) > 0.04);
  if (paints(getComputedStyle(el))) return true;
  const before = getComputedStyle(el, '::before');
  return before.content !== 'none' && before.content !== 'normal' && paints(before);
}

/** What confetti lands on for one control in the row: its own box when it
 *  draws one (the glass message box, Send), else its icon (the bare wallet
 *  and tools buttons), so nothing rests on an edge you cannot see. A
 *  wrapper's own control (the field's textarea, the wallet's button) carries
 *  the edge and its rounding. `glyph` keeps an earlier choice. */
function surfaceOf(
  el: Element,
  known?: { glyph: boolean; self: boolean },
): { rect: DOMRect; radius: number; glyph: boolean; self: boolean; icon: Element | null } | null {
  // A row child that draws its own box is the surface (the phone's message
  // pill wraps a clear textarea); otherwise its control is.
  const self = known?.self ?? (el.matches('button, textarea, input') || drawsBox(el));
  const control = self ? el : el.querySelector('textarea, input') ?? el.querySelector('button') ?? el;
  const icon = control.querySelector('svg, img');
  const onIcon = known?.glyph ?? (!drawsBox(control) && !!icon);
  if (onIcon) {
    if (!icon) return null;
    const rect = icon.getBoundingClientRect();
    // Icons are mostly round on top; resting toward the middle reads right.
    return { rect, radius: Math.min(rect.width, rect.height) / 2, glyph: true, self, icon };
  }
  const rect = control.getBoundingClientRect();
  return { rect, radius: parseFloat(getComputedStyle(control).borderTopLeftRadius) || 0, glyph: false, self, icon: null };
}

/** A perch's bare edge from its box: the flat top and its rounded corners. */
function boxEdge(p: Perch): Float32Array {
  const g = new Float32Array(Math.ceil(p.width / PILE_BIN));
  for (let b = 0; b < g.length; b++) {
    const s = surfaceAt(p, (b + 0.5) * PILE_BIN);
    g[b] = s === null ? NO_GROUND : -s;
  }
  return g;
}

/** An icon's drawn outline, read from its pixels: per PILE_BIN column, the
 *  height (above the icon box's top) of its highest opaque pixel. An icon's
 *  box has padding around the drawing, so its box edge would leave confetti
 *  floating over it. Null when the pixels cannot be read. */
async function iconOutline(icon: Element, width: number, height: number): Promise<Float32Array | null> {
  const w = Math.max(1, Math.round(width));
  const h = Math.max(1, Math.round(height));
  try {
    let src: CanvasImageSource;
    if (icon instanceof HTMLImageElement) {
      await icon.decode();
      src = icon;
    } else {
      const clone = icon.cloneNode(true) as SVGElement;
      clone.setAttribute('xmlns', 'http://www.w3.org/2000/svg');
      clone.setAttribute('width', String(w));
      clone.setAttribute('height', String(h));
      // currentColor resolves against the page; the copy needs it spelled out.
      clone.setAttribute('color', getComputedStyle(icon).color);
      const img = new Image();
      img.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(new XMLSerializer().serializeToString(clone))}`;
      await img.decode();
      src = img;
    }
    const c = document.createElement('canvas');
    c.width = w;
    c.height = h;
    const ctx = c.getContext('2d', { willReadFrequently: true });
    if (!ctx) return null;
    ctx.drawImage(src, 0, 0, w, h);
    const px = ctx.getImageData(0, 0, w, h).data;
    const out = new Float32Array(Math.ceil(w / PILE_BIN));
    for (let b = 0; b < out.length; b++) {
      let top = -1;
      for (let y = 0; y < h && top < 0; y++) {
        for (let x = b * PILE_BIN; x < Math.min(w, (b + 1) * PILE_BIN); x++) {
          if (px[(y * w + x) * 4 + 3] > 96) {
            top = y;
            break;
          }
        }
      }
      out[b] = top < 0 ? NO_GROUND : -top;
    }
    return out;
  } catch {
    return null; // undecodable or cross-origin: the box edge stands in
  }
}

/** The edge's tilt at local x, so a piece on a corner leans with it. */
function slopeAt(p: Perch, lx: number): number {
  const a = surfaceAt(p, lx - 1);
  const b = surfaceAt(p, lx + 1);
  if (a === null || b === null) return 0;
  return Math.atan2(b - a, 2);
}

/** Turn an angle difference into the nearest equivalent for a rectangle,
 *  which looks the same every half turn, so easing never spins it around. */
function nearestHalfTurn(from: number, to: number): number {
  let d = (to - from) % Math.PI;
  if (d > Math.PI / 2) d -= Math.PI;
  if (d < -Math.PI / 2) d += Math.PI;
  return from + d;
}

interface ConfettiBurstProps {
  golden?: boolean;
  /** Bump to throw another burst into this canvas, keeping what has landed. */
  burst?: number;
  /** Selector for a row inside the host whose direct children catch confetti. */
  perchOn?: string;
  /** Fired once nothing is falling or resting any more. */
  onDone?: () => void;
}

interface Engine {
  spawn: (colors: ColorPair[]) => void;
}

export default function ConfettiBurst({ golden = false, burst = 0, perchOn, onDone }: ConfettiBurstProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const engineRef = useRef<Engine | null>(null);
  const onDoneRef = useRef(onDone);
  const goldenRef = useRef(golden);
  // First, so the burst effect below already reads this render's colors.
  useEffect(() => {
    onDoneRef.current = onDone;
    goldenRef.current = golden;
  });

  useEffect(() => {
    const canvas = canvasRef.current;
    const parent = canvas?.parentElement;
    if (!canvas || !parent) return;

    if (window.matchMedia?.('(prefers-reduced-motion: reduce)').matches) {
      onDoneRef.current?.();
      return;
    }

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    let W = 0;
    let H = 0;
    const resize = () => {
      const dpr = window.devicePixelRatio || 1;
      W = parent.clientWidth;
      H = parent.clientHeight;
      canvas.width = Math.max(1, Math.round(W * dpr));
      canvas.height = Math.max(1, Math.round(H * dpr));
      canvas.style.width = `${W}px`;
      canvas.style.height = `${H}px`;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    };
    resize();

    let pieces: Piece[] = [];
    const perches = new Map<Element, Perch>();
    let raf = 0;
    let frame = 0;
    let finished = false;

    const observer = typeof ResizeObserver !== 'undefined'
      ? new ResizeObserver(() => {
          resize();
          measure();
          if (!raf) draw();
        })
      : null;
    observer?.observe(parent);

    // Pieces on a perch that is gone fall off it.
    const drop = (p: Piece) => {
      const at = restPosition(p);
      p.x = at.x;
      p.y = at.y;
      p.rot = at.rot;
      p.vx = rand(-1, 1);
      p.vy = 0;
      p.mode = 'fall';
      p.lands = false;
      p.perch = null;
      p.grow = SWEEP_GROW_FRAMES;
    };

    const measure = () => {
      if (!perchOn) return;
      const host = parent.getBoundingClientRect();
      const seen = new Set<Element>();
      parent.querySelectorAll(perchOn).forEach((row) => {
        for (const el of Array.from(row.children)) {
          const known = perches.get(el);
          const surface = surfaceOf(el, known);
          if (!surface) continue;
          const r = surface.rect;
          if (r.width < 4 || r.height < 4) continue;
          seen.add(el);
          let perch = known;
          if (!perch) {
            perch = {
              el, glyph: surface.glyph, self: surface.self, left: 0, top: 0, width: 0, height: 0, radius: 0,
              hit: { left: 0, top: 0, right: 0, bottom: 0 },
              base: new Float32Array(0), outline: null, ground: new Float32Array(0),
            };
            perches.set(el, perch);
            observer?.observe(el);
          }
          const radius = Math.min(surface.radius, r.width / 2, r.height / 2);
          const whole = el.getBoundingClientRect();
          perch.hit = {
            left: Math.min(whole.left, r.left) - host.left,
            top: Math.min(whole.top, r.top) - host.top,
            right: Math.max(whole.right, r.right) - host.left,
            bottom: Math.max(whole.bottom, r.bottom) - host.top,
          };
          perch.left = r.left - host.left;
          perch.top = r.top - host.top;
          perch.height = r.height;
          if (Math.abs(perch.width - r.width) > 0.5 || Math.abs(perch.radius - radius) > 0.5) {
            // A reshaped control keeps its confetti, but the pile restarts.
            perch.width = r.width;
            perch.radius = radius;
            perch.outline = null;
            perch.base = boxEdge(perch);
            perch.ground = perch.base.slice();
            if (perch.glyph && surface.icon) {
              const target = perch;
              const width = r.width;
              void iconOutline(surface.icon, r.width, r.height).then((outline) => {
                if (!outline || target.width !== width || !perches.has(target.el)) return;
                target.outline = outline;
                target.base = outline;
                // Nothing has landed yet in the moment this took: start clean.
                if (!pieces.some((p) => p.perch === target)) target.ground = outline.slice();
              });
            }
          }
        }
      });
      for (const [el, perch] of perches) {
        if (seen.has(el)) continue;
        perches.delete(el);
        observer?.unobserve(el);
        for (const p of pieces) if (p.perch === perch) drop(p);
        wake();
      }
    };

    const restPosition = (p: Piece) => {
      const perch = p.perch!;
      return { x: perch.left + p.offX, y: perch.top - p.lift, rot: p.restRot + Math.PI / 2 };
    };

    const restingCount = () => pieces.reduce((n, p) => n + (p.mode === 'rest' || p.mode === 'settle' ? 1 : 0), 0);

    const land = (p: Piece, perch: Perch, lx: number) => {
      const len = p.sequin ? 0 : p.h * 0.85;
      const thick = p.sequin ? p.w * 2 : rand(2, 4);
      const g = perch.ground;
      const dxOf = (b: number) => (b + 0.5) * PILE_BIN - lx;
      // Rest it as a rigid stick on the ground below: at angle `rot` its
      // underside is the line c - dx * tan, lowered until it touches the
      // highest point under it (at `touch`).
      const fit = (rot: number) => {
        const cos = Math.cos(rot);
        const tan = Math.tan(rot);
        const halfX = p.sequin ? p.w : (len / 2) * cos;
        const b0 = Math.max(0, Math.floor((lx - halfX) / PILE_BIN));
        const b1 = Math.min(g.length - 1, Math.floor((lx + halfX) / PILE_BIN));
        let c = NO_GROUND;
        let touch = 0;
        for (let b = b0; b <= b1; b++) {
          if (g[b] <= NO_GROUND) continue;
          const need = g[b] + dxOf(b) * tan;
          if (need > c) {
            c = need;
            touch = dxOf(b);
          }
        }
        return { rot, cos, tan, halfX, b0, b1, c, touch };
      };
      // Lying nearly flat along the edge (with its tilt on a corner).
      let f = fit(p.sequin ? 0 : slopeAt(perch, lx) + rand(-0.12, 0.12));
      if (f.c <= NO_GROUND) return false;
      if (!p.sequin && Math.abs(f.touch) > f.halfX / 3) {
        // Held up by one end (on a piece already there): it tips toward the
        // overhang until that end meets something, as real paper would.
        const h = f.c - f.touch * f.tan;
        const left = f.touch > 0;
        let m = left ? Infinity : -Infinity; // height change per px of the underside
        for (let b = 0; b < g.length; b++) {
          const dx = dxOf(b);
          if (g[b] <= NO_GROUND || Math.abs(dx) > len / 2 || (left ? dx >= f.touch : dx <= f.touch)) continue;
          const k = (h - g[b]) / (f.touch - dx);
          m = left ? Math.min(m, k) : Math.max(m, k);
        }
        const tipped = Math.max(-MAX_TIP, Math.min(MAX_TIP, Math.atan(-(Number.isFinite(m) ? m : left ? 99 : -99))));
        f = fit(tipped);
      }
      const { cos, tan, halfX, b0, b1, c } = f;
      // How high the pile already is here, over the bare edge.
      const bare = perch.base[Math.min(perch.base.length - 1, Math.max(0, Math.floor(lx / PILE_BIN)))];
      if (c - (bare <= NO_GROUND ? -MAX_CORNER_DROP : bare) > MAX_PILE) return false;
      const vThick = thick / cos;
      for (let b = b0; b <= b1; b++) {
        const dx = (b + 0.5) * PILE_BIN - lx;
        if (Math.abs(dx) > halfX) continue;
        g[b] = Math.max(g[b], c - dx * tan + vThick);
      }
      p.mode = 'settle';
      p.t = 0;
      p.perch = perch;
      p.offX = lx;
      p.lift = c + vThick / 2;
      p.restLen = len;
      p.restThick = thick;
      p.restRot = f.rot;
      p.fromW = p.w;
      p.fromH = p.h * p.scaleY;
      p.fromRot = p.rot;
      p.vx = 0;
      p.vy = 0;
      return true;
    };

    const isResting = (p: Piece) => p.mode === 'rest' || p.mode === 'settle';

    // Send one resting piece off its perch: it hops toward `dir` after `delay`
    // frames (the brush reaching it).
    const launch = (p: Piece, dir: number, delay: number) => {
      p.mode = 'swept';
      p.t = delay;
      p.vx = dir * rand(2.5, 5.5);
      p.vy = -rand(1.5, 3.2);
    };

    // Rebuild a perch's ground from what still lies on it, lowest first. A
    // piece that lay on one that left has nothing under it any more and slides
    // off the same way.
    const restack = (perch: Perch, dir: number) => {
      const g = perch.base.slice();
      const lying = pieces.filter((p) => p.perch === perch && isResting(p)).sort((a, b) => a.lift - b.lift);
      for (const p of lying) {
        const cos = Math.cos(p.restRot);
        const tan = Math.tan(p.restRot);
        const vThick = p.sequin ? p.w * 2 : p.restThick / cos;
        const halfX = p.sequin ? p.w : (p.restLen / 2) * cos;
        const under = p.lift - vThick / 2; // underside height at its center
        const b0 = Math.max(0, Math.floor((p.offX - halfX) / PILE_BIN));
        const b1 = Math.min(g.length - 1, Math.floor((p.offX + halfX) / PILE_BIN));
        let held = false;
        for (let b = b0; b <= b1 && !held; b++) {
          const dx = (b + 0.5) * PILE_BIN - p.offX;
          if (Math.abs(dx) <= halfX + PILE_BIN / 2 && g[b] > NO_GROUND && g[b] >= under - dx * tan - 1.5) held = true;
        }
        if (!held) {
          launch(p, dir || (Math.random() < 0.5 ? -1 : 1), 0);
          continue;
        }
        for (let b = b0; b <= b1; b++) {
          const dx = (b + 0.5) * PILE_BIN - p.offX;
          if (Math.abs(dx) <= halfX) g[b] = Math.max(g[b], under - dx * tan + vThick);
        }
      }
      perch.ground = g;
    };

    // Clear a whole perch (a tap, a focus): nearest `fromX` first, each piece
    // scattering away from it.
    const sweep = (perch: Perch, fromX: number) => {
      let any = false;
      for (const p of pieces) {
        if (p.perch !== perch || !isResting(p)) continue;
        const dx = restPosition(p).x - fromX;
        const dir = Math.abs(dx) < 4 ? (Math.random() < 0.5 ? -1 : 1) : Math.sign(dx);
        launch(p, dir, Math.min(SWEEP_MAX_DELAY, Math.floor(Math.abs(dx) / SWEEP_PX_PER_FRAME)));
        any = true;
      }
      if (any) {
        perch.ground = perch.base.slice();
        wake();
      }
    };

    // The pointer passing over the composer flicks off only the pieces right
    // around it, the way it is moving (`heading`, -1 or 1, or 0 when it only
    // moved up or down: then away from it), so a sweep follows the cursor.
    const brush = (cx: number, cy: number, heading: number) => {
      if (!perches.size) return;
      const host = parent.getBoundingClientRect();
      const x = cx - host.left;
      const y = cy - host.top;
      const touched = new Set<Perch>();
      for (const p of pieces) {
        if (!p.perch || !isResting(p)) continue;
        const at = restPosition(p);
        const dx = at.x - x;
        if (Math.abs(dx) > BRUSH_RADIUS || Math.abs(at.y - y) > BRUSH_RADIUS) continue;
        if (dx * dx + (at.y - y) * (at.y - y) > BRUSH_RADIUS * BRUSH_RADIUS) continue;
        launch(p, heading || (Math.abs(dx) < 2 ? (Math.random() < 0.5 ? -1 : 1) : Math.sign(dx)), 0);
        touched.add(p.perch);
      }
      if (!touched.size) return;
      for (const perch of touched) restack(perch, heading);
      wake();
    };

    // A tap clears the controls under it: anywhere on one, or just above it,
    // over its confetti.
    const sweepAt = (cx: number, cy: number) => {
      if (!perches.size || !restingCount()) return;
      const host = parent.getBoundingClientRect();
      const x = cx - host.left;
      const y = cy - host.top;
      for (const perch of perches.values()) {
        const { hit } = perch;
        if (
          x >= hit.left - 2 && x <= hit.right + 2 &&
          y >= Math.min(hit.top, perch.top) - REACH_ABOVE && y <= hit.bottom + 2
        ) {
          sweep(perch, x);
        }
      }
    };
    let lastX: number | null = null;
    // A finger or pen that went down: a tap if it lifts without moving (it
    // clears the control it touched), a swipe once it moves (it only brushes,
    // like the mouse), so a swipe along the composer never clears more than
    // it passes over.
    let touch: { x: number; y: number; moved: boolean } | null = null;
    // A mouse brushes as it hovers; a finger or pen brushes as it drags
    // across the controls (it has no hover).
    const onHover = (e: PointerEvent) => {
      if (e.pointerType !== 'mouse') {
        if (e.buttons === 0) return;
        if (touch && !touch.moved && Math.hypot(e.clientX - touch.x, e.clientY - touch.y) > TAP_SLOP) touch.moved = true;
        if (touch && !touch.moved) return;
      }
      const moved = lastX === null ? 0 : e.clientX - lastX;
      lastX = e.clientX;
      if (!restingCount()) return;
      brush(e.clientX, e.clientY, Math.abs(moved) >= 1 ? Math.sign(moved) : 0);
    };
    // A click clears the control it lands on at once; a finger waits to see
    // whether it is a tap.
    const onPress = (e: PointerEvent | MouseEvent) => {
      lastX = e.clientX;
      if ('pointerType' in e && e.pointerType !== 'mouse') {
        touch = { x: e.clientX, y: e.clientY, moved: false };
        return;
      }
      sweepAt(e.clientX, e.clientY);
    };
    const onRelease = (e: PointerEvent) => {
      if (e.pointerType === 'mouse') return;
      if (touch && !touch.moved) sweepAt(touch.x, touch.y);
      touch = null;
    };
    const onCancel = () => {
      touch = null;
    };
    // Reaching a control from the keyboard (Tab into the field) sweeps it too.
    const onFocus = (e: FocusEvent) => {
      const target = e.target as Node | null;
      if (!target) return;
      for (const perch of perches.values()) {
        if (perch.el.contains(target)) sweep(perch, perch.left + perch.width / 2);
      }
    };
    // A disabled button (Send with an empty box) still sweeps: Chromium gives
    // it pointer events but no mousedown, other engines may differ, so both
    // presses are heard. Whichever comes second finds nothing left to sweep.
    window.addEventListener('pointermove', onHover, { capture: true, passive: true });
    window.addEventListener('pointerup', onRelease, true);
    window.addEventListener('pointercancel', onCancel, true);
    window.addEventListener('pointerdown', onPress, true);
    window.addEventListener('mousedown', onPress, true);
    parent.addEventListener('focusin', onFocus);

    const drawPaper = (x: number, y: number, rot: number, w: number, h: number, fill: string) => {
      ctx.save();
      ctx.translate(x, y);
      ctx.rotate(rot);
      ctx.fillStyle = fill;
      ctx.fillRect(-w / 2, -h / 2, w, h);
      ctx.restore();
    };
    const drawSequin = (x: number, y: number, r: number, fill: string) => {
      ctx.fillStyle = fill;
      ctx.beginPath();
      ctx.arc(x, y, r, 0, 2 * Math.PI);
      ctx.fill();
    };

    const drawPiece = (p: Piece) => {
      if (p.mode === 'rest' || (p.mode === 'swept' && p.t > 0)) {
        const at = restPosition(p);
        if (p.sequin) drawSequin(at.x, at.y, p.w, p.color.back);
        else drawPaper(at.x, at.y, at.rot, p.restThick, p.restLen, p.color.front);
        return;
      }
      if (p.mode === 'settle') {
        const k = p.t / SETTLE_FRAMES;
        const ease = 1 - (1 - k) * (1 - k);
        const at = restPosition(p);
        const hop = SETTLE_BOUNCE * Math.sin(Math.PI * k) * (1 - k);
        if (p.sequin) {
          drawSequin(at.x, at.y - hop, p.w, p.color.back);
          return;
        }
        const toRot = nearestHalfTurn(p.fromRot, at.rot);
        drawPaper(
          at.x,
          at.y - hop,
          lerp(p.fromRot, toRot, ease),
          lerp(p.fromW, p.restThick, ease),
          lerp(Math.abs(p.fromH), p.restLen, ease),
          p.color.front,
        );
        return;
      }
      if (p.sequin) {
        drawSequin(p.x, p.y, p.w, p.color.back);
        return;
      }
      let w = p.w;
      let h = p.h * p.scaleY;
      if (p.grow > 0) {
        // Just left a perch: unfold from the flat sliver back to paper.
        const k = 1 - p.grow / SWEEP_GROW_FRAMES;
        w = lerp(p.restThick || p.w, p.w, k);
        h = lerp(p.restLen || p.h, p.h * p.scaleY, k);
      }
      drawPaper(p.x, p.y, p.rot, w, h, p.scaleY > 0 ? p.color.front : p.color.back);
    };

    const draw = () => {
      ctx.clearRect(0, 0, W, H);
      for (const p of pieces) drawPiece(p);
    };

    const step = (p: Piece) => {
      if (p.mode === 'rest') return;
      if (p.mode === 'settle') {
        if (++p.t >= SETTLE_FRAMES) p.mode = 'rest';
        return;
      }
      if (p.mode === 'swept') {
        if (p.t > 0) {
          p.t--;
          return;
        }
        // The brush reached it: launch from where it lay.
        const at = restPosition(p);
        p.x = at.x;
        p.y = at.y;
        p.rot = at.rot;
        // Start the flip edge-on, so the paper opens up instead of popping.
        p.flip = Math.PI / 2 / 0.09 - p.y;
        p.mode = 'fall';
        p.lands = false;
        p.perch = null;
        p.grow = SWEEP_GROW_FRAMES;
        return;
      }

      const prevY = p.y;
      if (p.sequin) {
        p.vx -= p.vx * DRAG_SEQUINS;
        p.vy += GRAVITY_SEQUINS;
      } else {
        p.vx -= p.vx * DRAG_CONFETTI;
        p.vy = Math.min(p.vy + GRAVITY_CONFETTI, TERMINAL_VELOCITY);
        p.vx += Math.random() > 0.5 ? Math.random() : -Math.random();
      }
      p.x += p.vx;
      p.y += p.vy;
      if (p.grow > 0) p.grow--;
      if (!p.sequin) {
        // .09 slows the flip frequency
        p.scaleY = Math.cos((p.y + p.flip) * 0.09);
        if (p.grow > 0) p.rot += p.vx * 0.04;
      }

      if (!p.lands || p.vy <= 0 || !perches.size) return;
      for (const perch of perches.values()) {
        const lx = p.x - perch.left;
        if (lx < 0 || lx >= perch.width || !perch.ground.length) continue;
        const ground = perch.ground[Math.min(perch.ground.length - 1, Math.floor(lx / PILE_BIN))];
        if (ground <= NO_GROUND) continue;
        const restY = perch.top - ground - 1.5;
        if (prevY <= restY && p.y >= restY) {
          if (restingCount() >= MAX_RESTING || !land(p, perch, lx)) p.lands = false;
          return;
        }
      }
    };

    const render = () => {
      raf = 0;
      frame++;
      if (frame % REMEASURE_EVERY === 0) measure();
      for (const p of pieces) step(p);
      pieces = pieces.filter((p) => p.mode !== 'fall' || p.y < H + 20);
      draw();

      if (!pieces.length) {
        if (!finished) {
          finished = true;
          onDoneRef.current?.();
        }
        return;
      }
      // Everything resting: stop the loop; resizes redraw it.
      if (pieces.some((p) => p.mode !== 'rest')) raf = requestAnimationFrame(render);
    };

    function wake() {
      finished = false;
      if (!raf) raf = requestAnimationFrame(render);
    }

    engineRef.current = {
      spawn: (colors) => {
        if (W === 0 || H === 0) return;
        measure();
        // Erupt from the upper area (around the hype bar) and rain down the rest.
        const originY = Math.min(H * 0.16, 110);
        const spread = Math.max(40, W * 0.28);
        const spawnX = () => rand(W / 2 - spread, W / 2 + spread);
        const spawnY = () => originY + rand(-6, 8);
        const base = {
          mode: 'fall' as Mode, t: 0, perch: null, offX: 0, lift: 0, restRot: 0,
          restLen: 0, restThick: 0, fromW: 0, fromH: 0, fromRot: 0, grow: 0,
        };
        for (let i = 0; i < CONFETTI_COUNT; i++) {
          const v = initConfettoVelocity([-9, 9], [6, 11]);
          pieces.push({
            ...base,
            sequin: false,
            color: pick(colors),
            w: rand(5, 9),
            h: rand(8, 15),
            x: spawnX(),
            y: spawnY(),
            vx: v.x,
            vy: v.y,
            rot: rand(0, 2 * Math.PI),
            flip: rand(0, 99),
            scaleY: 1,
            lands: Math.random() < LAND_CHANCE_CONFETTI,
          });
        }
        for (let i = 0; i < SEQUIN_COUNT; i++) {
          const color = pick(colors);
          pieces.push({
            ...base,
            sequin: true,
            color,
            w: rand(1, 2),
            h: 0,
            x: spawnX(),
            y: spawnY(),
            vx: rand(-6, 6),
            vy: -rand(8, 12),
            rot: 0,
            flip: 0,
            scaleY: 1,
            lands: Math.random() < LAND_CHANCE_SEQUINS,
          });
        }
        wake();
      },
    };

    return () => {
      engineRef.current = null;
      cancelAnimationFrame(raf);
      observer?.disconnect();
      window.removeEventListener('pointermove', onHover, true);
      window.removeEventListener('pointerup', onRelease, true);
      window.removeEventListener('pointercancel', onCancel, true);
      window.removeEventListener('pointerdown', onPress, true);
      window.removeEventListener('mousedown', onPress, true);
      parent.removeEventListener('focusin', onFocus);
    };
  }, [perchOn]);

  // Each burst (the first on mount, then every bump) throws a fresh handful.
  useEffect(() => {
    engineRef.current?.spawn(goldenRef.current ? GOLD : RAINBOW);
  }, [burst, perchOn]);

  return <canvas ref={canvasRef} className="absolute inset-0 z-30 pointer-events-none" aria-hidden="true" />;
}
