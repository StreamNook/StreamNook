import { describe, expect, it } from 'vitest';
import { createChatPerfSampler } from './chatPerfSampler';

const T0 = 1_000_000;
const MINUTE = 60_000;

/** Rows of the given ages, all noted at the start of the minute. */
function sampled(ages: number[], rng?: () => number) {
  const s = createChatPerfSampler(rng);
  for (const a of ages) s.noteRowAge(a, T0);
  return s;
}

describe('chatPerfSampler', () => {
  it('writes nothing before a minute of rows', () => {
    const s = sampled([100, 200]);
    expect(s.take('main', T0 + MINUTE - 1)).toBeNull();
  });

  it('writes a baseline line first, then stays quiet while chat is healthy', () => {
    const ages = Array.from({ length: 10 }, (_, i) => (i + 1) * 10);
    const s = sampled(ages);
    const first = s.take('main', T0 + MINUTE);
    expect(first).toContain('[ChatPerf] main: 10 live rows');
    expect(first).toContain('age p50 60ms p95 100ms max 100ms');
    expect(first).not.toContain('slow');

    for (const a of ages) s.noteRowAge(a, T0 + MINUTE);
    expect(s.take('main', T0 + 2 * MINUTE)).toBeNull();
  });

  it('writes the baseline again after ten minutes', () => {
    const s = sampled([50]);
    s.take('main', T0 + MINUTE);
    const at = T0 + 11 * MINUTE;
    s.noteRowAge(50, at - MINUTE);
    expect(s.take('main', at)).toContain('[ChatPerf] main:');
  });

  it('marks a minute slow when rows spread past a second', () => {
    const s = sampled([...Array.from({ length: 19 }, () => 200), 1500]);
    s.take('main', T0 + MINUTE);
    for (const a of [...Array.from({ length: 19 }, () => 200), 1500, 1500]) s.noteRowAge(a, T0 + MINUTE);
    const line = s.take('main', T0 + 2 * MINUTE);
    expect(line).toContain('[ChatPerf] main slow:');
    expect(line).toContain('p95 over fastest 1300ms');
  });

  it('marks a minute slow on a long paint wait or a long frame', () => {
    const paint = sampled([100]);
    paint.take('main', T0 + MINUTE);
    paint.noteRowAge(100, T0 + MINUTE);
    paint.notePaintWait(600);
    expect(paint.take('main', T0 + 2 * MINUTE)).toContain('slow');

    const frame = sampled([100]);
    frame.take('main', T0 + MINUTE);
    frame.noteRowAge(100, T0 + MINUTE);
    frame.noteLongFrame(250, 'WebSocket.onmessage@120');
    expect(frame.take('main', T0 + 2 * MINUTE)).toContain('slow');
  });

  it('ignores ages outside the live window', () => {
    const s = sampled([-20_000, 90_000, Number.NaN, 300]);
    expect(s.take('main', T0 + MINUTE)).toContain('1 live rows');
  });

  it('keeps a fixed sample of a busy room but the true max', () => {
    const ages = Array.from({ length: 3000 }, (_, i) => (i === 2999 ? 9000 : 100));
    const s = sampled(ages, () => 0.999);
    const line = s.take('main', T0 + MINUTE);
    expect(line).toContain('3000 live rows');
    expect(line).toContain('max 9000ms');
  });

  it('names the slowest scripts first', () => {
    const s = sampled([100]);
    s.noteLongFrame(150, 'IMG.onerror@40');
    s.noteLongFrame(400, 'WebSocket.onmessage@120');
    s.noteLongFrame(120, null);
    const line = s.take('main', T0 + MINUTE);
    expect(line).toContain('long frames 3 worst 400ms (WebSocket.onmessage@120 400ms, IMG.onerror@40 150ms)');
  });

  it('starts every minute clean', () => {
    const s = sampled([100]);
    s.notePaintWait(900);
    s.noteLongFrame(300, 'x@1');
    s.take('main', T0 + MINUTE);
    expect(s.take('main', T0 + 2 * MINUTE)).toBeNull();
    s.noteRowAge(100, T0 + 2 * MINUTE);
    expect(s.take('main', T0 + 3 * MINUTE)).toBeNull();
  });
});
