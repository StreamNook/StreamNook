// The listening bars' motion: an auto-gain envelope so a quiet mix moves them
// as much as a loud one, attack and release smoothing scaled by the real frame
// time, a ripple so a steady level still flows, a wave while the song is
// looked up, and the Motion setting honored.
import { describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/event', () => ({ emitTo: vi.fn() }));
vi.mock('@tauri-apps/api/webviewWindow', () => ({ getCurrentWebviewWindow: () => ({ label: 'main', listen: vi.fn() }) }));

import {
  ATTACK,
  BAR_WEIGHTS,
  barScales,
  PEAK_FLOOR,
  PEAK_HALF_LIFE_MS,
  QUIET,
  RELEASE,
  STILL_BARS,
  stepEnvelope,
  type Envelope,
} from './listeningBars';

const run = (env: Envelope, level: number, ms: number, frame = 1000 / 60): Envelope => {
  let e = env;
  for (let t = 0; t < ms; t += frame) e = stepEnvelope(e, level, frame);
  return e;
};

describe('the auto-gain envelope', () => {
  it('lifts a quiet mix to the same height as a loud one', () => {
    const quiet = run(QUIET, 0.06, 1000);
    const loud = run(QUIET, 0.3, 1000);
    expect(quiet.level).toBeGreaterThan(0.95);
    expect(loud.level).toBeGreaterThan(0.95);
  });

  it('remembers the loudest moment, halving it every 2.5 s', () => {
    const peaked = stepEnvelope({ peak: PEAK_FLOOR, level: 0 }, 0.4, 16);
    expect(peaked.peak).toBeCloseTo(0.4, 6);
    const later = stepEnvelope(peaked, 0, PEAK_HALF_LIFE_MS);
    // One step is capped at 250 ms (a stalled frame does not drop it at once).
    expect(later.peak).toBeCloseTo(0.4 * 0.5 ** (250 / PEAK_HALF_LIFE_MS), 6);
    expect(run(peaked, 0, PEAK_HALF_LIFE_MS, 50).peak).toBeCloseTo(0.2, 2);
  });

  it('never falls under its floor, so silence stays down', () => {
    const silent = run({ peak: 0.5, level: 1 }, 0, 60_000, 100);
    expect(silent.peak).toBe(PEAK_FLOOR);
    expect(silent.level).toBeLessThan(0.01);
  });
});

describe('the smoothing', () => {
  it('rises fast and falls softly', () => {
    const up = stepEnvelope({ peak: 1, level: 0 }, 1, 1000 / 60);
    expect(up.level).toBeCloseTo(ATTACK, 6);
    const down = stepEnvelope({ peak: 1, level: 1 }, 0, 1000 / 60);
    expect(down.level).toBeCloseTo(1 - RELEASE, 6);
  });

  it('moves as far per second at any frame rate', () => {
    const at60 = run({ peak: 1, level: 0 }, 1, 100, 1000 / 60);
    const at30 = run({ peak: 1, level: 0 }, 1, 100, 1000 / 30);
    expect(Math.abs(at60.level - at30.level)).toBeLessThan(0.02);
  });
});

describe('the bars', () => {
  it('follow the level by weight, and a steady level still ripples', () => {
    const a = barScales('level', 1, 0, 'full');
    const b = barScales('level', 1, 0.37, 'full');
    expect(a).toHaveLength(5);
    expect(a.some((v, i) => Math.abs(v - b[i]) > 0.02)).toBe(true);
    for (const v of a) {
      expect(v).toBeGreaterThanOrEqual(0.18);
      expect(v).toBeLessThanOrEqual(1);
    }
    // Silence rests at the floor.
    expect(barScales('level', 0, 1, 'full').every((v) => Math.abs(v - 0.18) < 1e-9)).toBe(true);
  });

  it('run a slow wave while the song is looked up, with no level', () => {
    const a = barScales('finding', 0, 0, 'full');
    const b = barScales('finding', 0, 0.3, 'full');
    expect(a.some((v, i) => Math.abs(v - b[i]) > 0.02)).toBe(true);
    expect(new Set(a.map((v) => v.toFixed(3))).size).toBeGreaterThan(1);
  });

  it('Reduced Motion: the level still sets their height, with no ripple and no wave', () => {
    const a = barScales('level', 0.8, 0, 'reduced');
    expect(a).toEqual(barScales('level', 0.8, 5, 'reduced'));
    expect(a).toEqual(BAR_WEIGHTS.map((w) => 0.18 + 0.82 * 0.8 * w));
    expect(barScales('finding', 0, 3, 'reduced')).toEqual([...STILL_BARS]);
  });

  it('Motion off: a still shape', () => {
    for (const state of ['breathing', 'level', 'finding'] as const) {
      expect(barScales(state, 0.7, 2, 'off')).toEqual([0.4, 0.7, 1, 0.7, 0.4]);
    }
  });
});
