// Every finger on the watch band (SN-30), so two fingers can pinch the picture
// without a resting thumb ever turning a dial or a swipe into a zoom. Pure:
// WatchScreen feeds it pointer events and acts on what it answers. One finger
// at a time is the `driver` (WatchScreen's single-finger gesture follows it);
// a thumb that rests or was let go is a `rest`; extra fingers are `spare`.
import { pinchDecision, pinchPreviewScale, pinchTick } from '../player/videoFill';

export const TOUCH_SLOP = 8;
export const PINCH_WINDOW_MS = 300;
export const PINCH_MIN_SPAN = 24;

export type DriverKind = 'dial' | 'swipe' | 'shrink' | 'move';
type Role = 'driver' | 'rest' | 'pinch' | 'spare';

interface Finger {
  x: number;
  y: number;
  x0: number;
  y0: number;
  t0: number;
  moved: boolean;
  role: Role;
  kind: DriverKind | null;
}

interface Pinch {
  a: number;
  b: number;
  d0: number;
  startFill: boolean;
  fill: number;
  armed: boolean;
  ratio: number;
}

export interface TouchPoint {
  id: number;
  x: number;
  y: number;
  /** The event's timeStamp, in ms. */
  t: number;
  primary: boolean;
  /** The press landed inside the band's own DOM (not a portaled sheet). */
  own: boolean;
}

export interface PinchContext {
  pinchable: boolean;
  fill: number;
  startFill: boolean;
}

export type DownAnswer =
  | { kind: 'ignore' }
  | { kind: 'single'; endedPinch: boolean }
  | { kind: 'single-restart'; abandon: boolean }
  | { kind: 'pinch-start' };

export type MoveAnswer = { kind: 'ignore' } | { kind: 'single' } | { kind: 'pinch'; scale: number; tick: boolean };

export type UpAnswer =
  | { kind: 'ignore' }
  | { kind: 'single' }
  | { kind: 'pinch-end'; want: boolean; startFill: boolean }
  | { kind: 'pinch-cancel' };

export interface BandTouches {
  down(p: TouchPoint, ctx: PinchContext): DownAnswer;
  move(p: { id: number; x: number; y: number }): MoveAnswer;
  up(id: number): UpAnswer;
  cancel(id: number): UpAnswer;
  /** What the driver became at the slop. */
  latch(id: number, kind: DriverKind): void;
  /** The driver's gesture ended while the finger is still down: it rests. */
  drop(id: number): void;
  /** The two pinching fingers, for pointer capture. */
  pinchIds(): number[];
  /** Ends a pinch in flight with no change; true when there was one. */
  abortPinch(): boolean;
  /** Forgets every finger; true when a pinch was running. */
  reset(): boolean;
}

export function createBandTouches(): BandTouches {
  const fingers = new Map<number, Finger>();
  let pinch: Pinch | null = null;
  let locked = false;

  const add = (p: TouchPoint, role: Role) =>
    fingers.set(p.id, { x: p.x, y: p.y, x0: p.x, y0: p.y, t0: p.t, moved: false, role, kind: null });
  const endPinch = () => {
    pinch = null;
    locked = true;
    for (const f of fingers.values()) f.role = 'spare';
  };
  const lift = (id: number, cancelled: boolean): UpAnswer => {
    const f = fingers.get(id);
    if (!f) return { kind: 'ignore' };
    fingers.delete(id);
    let answer: UpAnswer;
    if (f.role === 'pinch' && pinch) {
      answer = cancelled
        ? { kind: 'pinch-cancel' }
        : {
            kind: 'pinch-end',
            want: pinchDecision(pinch.startFill, pinch.fill, pinch.ratio),
            startFill: pinch.startFill,
          };
      endPinch();
    } else {
      answer = f.role === 'driver' ? { kind: 'single' } : { kind: 'ignore' };
    }
    if (fingers.size === 0) locked = false;
    return answer;
  };

  return {
    down(p, ctx) {
      if (!p.own) return { kind: 'ignore' };
      if (p.primary) {
        const endedPinch = pinch !== null;
        fingers.clear();
        pinch = null;
        locked = false;
        add(p, 'driver');
        return { kind: 'single', endedPinch };
      }
      if (pinch || locked) {
        add(p, 'spare');
        return { kind: 'ignore' };
      }
      const held = [...fingers.entries()];
      if (held.every(([, f]) => f.role === 'rest')) {
        add(p, 'driver');
        return { kind: 'single', endedPinch: false };
      }
      const drivers = held.filter(([, f]) => f.role === 'driver');
      const others = held.filter(([, f]) => f.role !== 'driver' && f.role !== 'rest');
      if (drivers.length === 1 && others.length === 0) {
        const [driverId, driver] = drivers[0];
        const span = Math.hypot(p.x - driver.x, p.y - driver.y);
        if (ctx.pinchable && p.t - driver.t0 <= PINCH_WINDOW_MS && span >= PINCH_MIN_SPAN) {
          add(p, 'pinch');
          driver.role = 'pinch';
          pinch = { a: driverId, b: p.id, d0: span, startFill: ctx.startFill, fill: ctx.fill, armed: true, ratio: 1 };
          return { kind: 'pinch-start' };
        }
        if (!driver.moved || driver.kind === 'swipe') {
          const abandon = driver.kind === 'swipe';
          driver.role = 'rest';
          add(p, 'driver');
          return { kind: 'single-restart', abandon };
        }
      }
      add(p, 'spare');
      return { kind: 'ignore' };
    },
    move(p) {
      const f = fingers.get(p.id);
      if (!f) return { kind: 'ignore' };
      f.x = p.x;
      f.y = p.y;
      if (!f.moved && (Math.abs(p.x - f.x0) > TOUCH_SLOP || Math.abs(p.y - f.y0) > TOUCH_SLOP)) f.moved = true;
      if (f.role === 'pinch' && pinch) {
        const a = fingers.get(pinch.a);
        const b = fingers.get(pinch.b);
        if (!a || !b) return { kind: 'ignore' };
        pinch.ratio = Math.hypot(a.x - b.x, a.y - b.y) / pinch.d0;
        const step = pinchTick(pinch.armed, pinch.startFill, pinch.fill, pinch.ratio);
        pinch.armed = step.armed;
        return { kind: 'pinch', scale: pinchPreviewScale(pinch.startFill, pinch.fill, pinch.ratio), tick: step.tick };
      }
      return f.role === 'driver' ? { kind: 'single' } : { kind: 'ignore' };
    },
    up: (id) => lift(id, false),
    cancel: (id) => lift(id, true),
    latch(id, kind) {
      const f = fingers.get(id);
      if (f && f.role === 'driver') f.kind = kind;
    },
    drop(id) {
      const f = fingers.get(id);
      if (f && f.role === 'driver') f.role = 'rest';
    },
    pinchIds: () => (pinch ? [pinch.a, pinch.b] : []),
    abortPinch() {
      if (!pinch) return false;
      endPinch();
      return true;
    },
    reset() {
      const was = pinch !== null;
      fingers.clear();
      pinch = null;
      locked = false;
      return was;
    },
  };
}
