import { describe, expect, it } from 'vitest';
import { createStandDownWatch, resyncOnResume, surfaceLossAction } from './pausePolicy';
import { noteViewerPause, takeViewerPause } from './viewerPause';

describe('pausePolicy', () => {
  it('resyncs only after 30 s paused', () => {
    expect(resyncOnResume(null, 1e9)).toBe(false);
    expect(resyncOnResume(0, 29_999)).toBe(false);
    expect(resyncOnResume(0, 30_000)).toBe(true);
  });

  it('an interruption (a call, another app sounding) comes back at the live edge after 4 s (SN-287)', () => {
    expect(resyncOnResume(0, 3_999, false)).toBe(false);
    expect(resyncOnResume(0, 4_000, false)).toBe(true);
    expect(resyncOnResume(0, 20_000, false)).toBe(true);
    // The viewer's own pause keeps their place until the relay parks.
    expect(resyncOnResume(0, 20_000, true)).toBe(false);
    expect(resyncOnResume(null, 1e9, false)).toBe(false);
  });

  it('only a pause noted just before it counts as the viewer chose it', () => {
    expect(takeViewerPause(1_000)).toBe(false);
    noteViewerPause(1_000);
    expect(takeViewerPause(1_200)).toBe(true);
    // Reading spends the note: the next pause (a call) is not the viewer's.
    expect(takeViewerPause(5_000)).toBe(false);
    noteViewerPause(1_000);
    expect(takeViewerPause(2_500)).toBe(false);
  });

  it('surface loss: PiP or paused does nothing; background play decides', () => {
    expect(surfaceLossAction({ inPip: true, playing: true, backgroundPlay: false })).toBe('none');
    expect(surfaceLossAction({ inPip: false, playing: false, backgroundPlay: false })).toBe('none');
    expect(surfaceLossAction({ inPip: false, playing: true, backgroundPlay: true })).toBe('downshift');
    expect(surfaceLossAction({ inPip: false, playing: true, backgroundPlay: false })).toBe('pause');
  });

  // A fake wall clock plus a manual queue standing in for setTimeout.
  function harness() {
    const h = {
      now: 0,
      due: 0,
      queue: [] as Array<() => void>,
    };
    const watch = createStandDownWatch({
      now: () => h.now,
      schedule: (fn) => {
        h.queue.push(fn);
        return () => {
          const i = h.queue.indexOf(fn);
          if (i >= 0) h.queue.splice(i, 1);
        };
      },
      onDue: () => {
        h.due += 1;
      },
    });
    const runNext = () => h.queue.shift()?.();
    return { h, watch, runNext };
  }

  it('stand-down fires once after 10 min of wall clock', () => {
    const { h, watch, runNext } = harness();
    watch.arm();
    h.now = 9 * 60_000;
    runNext();
    expect(h.due).toBe(0);
    expect(h.queue).toHaveLength(1);
    h.now = 10 * 60_000;
    runNext();
    expect(h.due).toBe(1);
    expect(watch.armed).toBe(false);
    expect(h.queue).toHaveLength(0);
  });

  it('catches up after deep sleep', () => {
    const { h, watch, runNext } = harness();
    watch.arm();
    h.now = 3 * 60 * 60_000;
    runNext();
    expect(h.due).toBe(1);
  });

  it('disarm cancels; re-arm keeps the first start', () => {
    const { h, watch, runNext } = harness();
    watch.arm();
    watch.disarm();
    expect(watch.armed).toBe(false);
    expect(h.queue).toHaveLength(0);

    watch.arm(); // starts at 0
    h.now = 5 * 60_000;
    watch.arm(); // already armed: the start stays at 0
    expect(h.queue).toHaveLength(1);
    h.now = 10 * 60_000;
    runNext();
    expect(h.due).toBe(1);
  });
});
