// The watch band's fingers (SN-30): two fingers pinch only when they land
// together; a resting thumb never turns a dial or a swipe into a zoom; presses
// from a portaled sheet never reach the band.
import { describe, expect, it } from 'vitest';
import { createBandTouches, type PinchContext } from './bandTouches';

const ctx = (over: Partial<PinchContext> = {}): PinchContext => ({ pinchable: true, fill: 1.25, startFill: false, ...over });
const pt = (id: number, x: number, y: number, t: number, primary = id === 1, own = true) => ({ id, x, y, t, primary, own });

describe('band fingers', () => {
  it('ignores a press from outside the band', () => {
    const b = createBandTouches();
    expect(b.down(pt(1, 0, 0, 0, true, false), ctx()).kind).toBe('ignore');
    expect(b.move({ id: 1, x: 50, y: 0 }).kind).toBe('ignore');
    expect(b.up(1).kind).toBe('ignore');
  });

  it('pinches when two fingers land together, and the first lift decides', () => {
    const b = createBandTouches();
    expect(b.down(pt(1, 100, 100, 0), ctx())).toEqual({ kind: 'single', endedPinch: false });
    expect(b.down(pt(2, 300, 100, 100), ctx())).toEqual({ kind: 'pinch-start' });
    expect(b.pinchIds()).toEqual([1, 2]);
    expect(b.move({ id: 2, x: 340, y: 100 })).toEqual({ kind: 'pinch', scale: 1.2, tick: true });
    expect(b.move({ id: 2, x: 345, y: 100 })).toMatchObject({ kind: 'pinch', tick: false });
    expect(b.up(2)).toEqual({ kind: 'pinch-end', want: true, startFill: false });
    expect(b.down(pt(3, 200, 100, 600), ctx()).kind).toBe('ignore');
    expect(b.up(1).kind).toBe('ignore');
    expect(b.up(3).kind).toBe('ignore');
    expect(b.down(pt(1, 100, 100, 900), ctx())).toEqual({ kind: 'single', endedPinch: false });
  });

  it('a later finger on a resting thumb takes over the gesture', () => {
    const b = createBandTouches();
    b.down(pt(1, 100, 100, 0), ctx());
    expect(b.down(pt(2, 300, 100, 400), ctx())).toEqual({ kind: 'single-restart', abandon: false });
    expect(b.move({ id: 1, x: 140, y: 100 }).kind).toBe('ignore');
    expect(b.move({ id: 2, x: 300, y: 160 }).kind).toBe('single');
    expect(b.up(1).kind).toBe('ignore');
    expect(b.up(2).kind).toBe('single');
  });

  it('a later finger on a latched swipe abandons it and drives', () => {
    const b = createBandTouches();
    b.down(pt(1, 100, 100, 0), ctx());
    expect(b.move({ id: 1, x: 130, y: 100 }).kind).toBe('single');
    b.latch(1, 'swipe');
    expect(b.down(pt(2, 300, 100, 500), ctx())).toEqual({ kind: 'single-restart', abandon: true });
  });

  it('a later finger while the first dials stays a spare', () => {
    const b = createBandTouches();
    b.down(pt(1, 100, 100, 0), ctx());
    b.move({ id: 1, x: 100, y: 140 });
    b.latch(1, 'dial');
    expect(b.down(pt(2, 300, 100, 500), ctx()).kind).toBe('ignore');
    expect(b.move({ id: 2, x: 320, y: 100 }).kind).toBe('ignore');
    expect(b.up(1).kind).toBe('single');
  });

  it('a dropped driver hands the next finger the gesture', () => {
    const b = createBandTouches();
    b.down(pt(1, 100, 100, 0), ctx());
    b.move({ id: 1, x: 130, y: 100 });
    b.drop(1);
    expect(b.down(pt(2, 300, 100, 500), ctx())).toEqual({ kind: 'single', endedPinch: false });
  });

  it('never pinches too close, too late, or where the picture cannot zoom', () => {
    const close = createBandTouches();
    close.down(pt(1, 100, 100, 0), ctx());
    expect(close.down(pt(2, 110, 105, 50), ctx()).kind).toBe('single-restart');
    const late = createBandTouches();
    late.down(pt(1, 100, 100, 0), ctx());
    expect(late.down(pt(2, 300, 100, 301), ctx()).kind).toBe('single-restart');
    const fixed = createBandTouches();
    fixed.down(pt(1, 100, 100, 0), ctx());
    expect(fixed.down(pt(2, 300, 100, 50), ctx({ pinchable: false })).kind).toBe('single-restart');
  });

  it('a finger that lands first outside the band drives', () => {
    const b = createBandTouches();
    expect(b.down(pt(2, 100, 100, 0, false), ctx())).toEqual({ kind: 'single', endedPinch: false });
  });

  it('a cancel ends a pinch with no change', () => {
    const b = createBandTouches();
    b.down(pt(1, 100, 100, 0), ctx());
    b.down(pt(2, 300, 100, 50), ctx());
    b.move({ id: 2, x: 400, y: 100 });
    expect(b.cancel(1)).toEqual({ kind: 'pinch-cancel' });
  });

  it('a primary press after a lost pinch reports it', () => {
    const b = createBandTouches();
    b.down(pt(1, 100, 100, 0), ctx());
    b.down(pt(2, 300, 100, 50), ctx());
    expect(b.down(pt(5, 200, 100, 4000, true), ctx())).toEqual({ kind: 'single', endedPinch: true });
  });

  it('a third finger during a pinch changes nothing', () => {
    const b = createBandTouches();
    b.down(pt(1, 100, 100, 0), ctx());
    b.down(pt(2, 300, 100, 50), ctx());
    expect(b.down(pt(3, 200, 300, 80), ctx()).kind).toBe('ignore');
    expect(b.move({ id: 3, x: 260, y: 300 }).kind).toBe('ignore');
    expect(b.move({ id: 2, x: 340, y: 100 })).toMatchObject({ kind: 'pinch', scale: 1.2 });
    expect(b.up(3).kind).toBe('ignore');
    expect(b.up(2)).toEqual({ kind: 'pinch-end', want: true, startFill: false });
  });

  it('reset reports a pinch that was running', () => {
    const b = createBandTouches();
    b.down(pt(1, 100, 100, 0), ctx());
    b.down(pt(2, 300, 100, 50), ctx());
    expect(b.reset()).toBe(true);
    expect(b.up(2).kind).toBe('ignore');
  });

  it('abortPinch and reset end a pinch', () => {
    const b = createBandTouches();
    b.down(pt(1, 100, 100, 0), ctx());
    b.down(pt(2, 300, 100, 50), ctx());
    expect(b.abortPinch()).toBe(true);
    expect(b.move({ id: 2, x: 400, y: 100 }).kind).toBe('ignore');
    expect(b.abortPinch()).toBe(false);
    expect(b.reset()).toBe(false);
  });

  it('nothing to fill never ticks', () => {
    const b = createBandTouches();
    b.down(pt(1, 100, 100, 0), ctx({ fill: 1 }));
    b.down(pt(2, 300, 100, 50), ctx({ fill: 1 }));
    expect(b.move({ id: 2, x: 500, y: 100 })).toMatchObject({ kind: 'pinch', tick: false });
    expect(b.up(2)).toEqual({ kind: 'pinch-end', want: false, startFill: false });
  });
});
