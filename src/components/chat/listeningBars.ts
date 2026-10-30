// The listening bars' one renderer, shared by every set of bars in the window
// (the listening strip over a chat's composer and the player's note). The
// stream's sound level arrives about 30 times a second (15 from another
// window); one requestAnimationFrame loop turns it into motion that flows:
//
// - an auto-gain envelope, so a quiet mix moves the bars as much as a loud
//   one (raw RMS sits around 0.05 to 0.3);
// - attack and release smoothing per frame, scaled by the real frame time;
// - five weighted bars, each with its own slow ripple, so a steady level still
//   flows like an equalizer.
//
// Before the first level the bars breathe; while the clip is looked up they
// run a slow traveling wave. Only `transform: scaleY` is written, straight to
// the bars (never React state), and the loop runs only while a listening
// exists and the window is in view. The Motion setting is honored: 'off'
// draws a still shape with no loop, 'reduced' follows the level with no
// ripple or wave.

import { getListening, subscribeLevel, subscribeListening, type Listening } from '../../utils/songListening';
import { isWindowHidden, onWindowVisibility } from '../../utils/windowVisibility';

export const BAR_WEIGHTS = [0.55, 0.85, 1, 0.8, 0.6] as const;
/** Each bar's ripple, rad/s, and where it starts. */
const RIPPLE_FREQ = [5.3, 7.4, 6.1, 8.6, 5.8];
const RIPPLE_PHASE = [0, 1.9, 3.4, 4.6, 2.6];
/** The shape the bars hold when nothing may move. */
export const STILL_BARS = [0.4, 0.7, 1, 0.7, 0.4] as const;

/** The envelope halves this often with no louder level. */
export const PEAK_HALF_LIFE_MS = 2500;
/** The envelope never falls under this, so silence stays near the floor. */
export const PEAK_FLOOR = 0.02;
/** Per-frame smoothing at 60 fps: up fast, down softly. */
export const ATTACK = 0.45;
export const RELEASE = 0.12;
const FRAME_MS = 1000 / 60;

export type MotionMode = 'full' | 'reduced' | 'off';

export interface Envelope {
  /** The loudest recent level, decaying. */
  peak: number;
  /** The smoothed, normalized level (0 to 1). */
  level: number;
}

export const QUIET: Envelope = { peak: PEAK_FLOOR, level: 0 };

/** The envelope after `dtMs` with the stream at `level`. */
export function stepEnvelope(env: Envelope, level: number, dtMs: number): Envelope {
  const dt = Math.max(0, Math.min(250, dtMs));
  const raw = Math.max(0, level);
  const peak = Math.max(PEAK_FLOOR, raw, env.peak * 0.5 ** (dt / PEAK_HALF_LIFE_MS));
  const target = Math.min(1, Math.max(0, raw / peak));
  const k = 1 - (1 - (target > env.level ? ATTACK : RELEASE)) ** (dt / FRAME_MS);
  return { peak, level: env.level + (target - env.level) * k };
}

/** What the bars show now. */
export type BarsState = 'breathing' | 'level' | 'finding';

/** Each bar's height (scaleY, 0 to 1) at `t` seconds. */
export function barScales(state: BarsState, level: number, t: number, mode: MotionMode): number[] {
  if (mode === 'off') return [...STILL_BARS];
  if (state === 'level') {
    return BAR_WEIGHTS.map((w, i) => {
      const ripple = mode === 'full' ? 0.72 + 0.28 * Math.sin(t * RIPPLE_FREQ[i] + RIPPLE_PHASE[i]) : 1;
      return 0.18 + 0.82 * level * w * ripple;
    });
  }
  if (mode === 'reduced') return [...STILL_BARS];
  // A slow wave across the bars: softer while it waits for the sound, a
  // little quicker while the song is looked up, so both read as working.
  const speed = state === 'finding' ? 3.4 : 2.4;
  const reach = state === 'finding' ? 0.5 : 0.38;
  return BAR_WEIGHTS.map((_, i) => 0.22 + reach * (0.5 + 0.5 * Math.sin(t * speed - i * 0.85)));
}

/** The window's Motion setting (MotionScope writes it on <html>). */
export function motionMode(): MotionMode {
  const m = typeof document === 'undefined' ? undefined : document.documentElement.dataset.motion;
  return m === 'off' || m === 'reduced' ? m : 'full';
}

// ---------------------------------------------------------------- the loop

const groups = new Set<HTMLElement[]>();
let env: Envelope = QUIET;
let level = 0;
let heard = false;
let listeningId: string | null = null;
let frame = 0;
let lastAt = 0;
let stops: (() => void)[] = [];

function paint(state: BarsState, t: number, mode: MotionMode): void {
  const scales = barScales(state, env.level, t, mode);
  for (const bars of groups) {
    bars.forEach((bar, i) => {
      bar.style.transform = `scaleY(${scales[i].toFixed(3)})`;
    });
  }
}

function stateOf(now: Listening | null): BarsState {
  if (now?.phase === 'finding') return 'finding';
  return heard ? 'level' : 'breathing';
}

function tick(at: number): void {
  frame = 0;
  const now = getListening();
  if (!now || !groups.size || isWindowHidden()) return;
  const mode = motionMode();
  const dt = lastAt ? at - lastAt : FRAME_MS;
  lastAt = at;
  env = stepEnvelope(env, now.phase === 'finding' ? 0 : level, dt);
  paint(stateOf(now), at / 1000, mode);
  // A still shape needs no loop: it is drawn again when the setting or the
  // listening changes.
  if (mode !== 'off') frame = requestAnimationFrame(tick);
}

/** Start the loop when there is something to draw, or draw the still shape. */
function wake(): void {
  const now = getListening();
  if (now?.id !== listeningId) {
    listeningId = now?.id ?? null;
    env = QUIET;
    level = 0;
    heard = false;
  }
  if (!now || !groups.size || isWindowHidden() || frame) return;
  lastAt = 0;
  frame = requestAnimationFrame(tick);
}

function start(): void {
  stops = [
    subscribeLevel((l) => {
      level = l;
      heard = true;
    }),
    subscribeListening(wake),
    onWindowVisibility(wake),
  ];
  // The Motion setting can change while bars are up.
  const observer = new MutationObserver(wake);
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ['data-motion'] });
  stops.push(() => observer.disconnect());
}

function stop(): void {
  for (const s of stops) s();
  stops = [];
  if (frame) cancelAnimationFrame(frame);
  frame = 0;
}

/** Draw a set of bars (their elements, in order) until the returned stop. */
export function drawBars(bars: HTMLElement[]): () => void {
  groups.add(bars);
  if (groups.size === 1) start();
  // Drawn at once in its present state, then by the loop.
  paint(stateOf(getListening()), performance.now() / 1000, motionMode());
  wake();
  return () => {
    groups.delete(bars);
    if (!groups.size) stop();
  };
}
