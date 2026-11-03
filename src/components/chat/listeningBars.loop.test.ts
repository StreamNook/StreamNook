// When the listening bars' loop runs: always in Full, and under Reduced only
// while the bars follow the level. Breathing under Reduced draws once and
// rests, and the first heard level starts the loop again.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/event', () => ({ emitTo: vi.fn() }));
vi.mock('@tauri-apps/api/webviewWindow', () => ({ getCurrentWebviewWindow: () => ({ label: 'main', listen: vi.fn() }) }));

const h = vi.hoisted(() => ({
  listening: { id: 'a', phase: 'listening' } as { id: string; phase: 'listening' | 'finding' } | null,
  level: null as null | ((l: number) => void),
}));

vi.mock('../../utils/songListening', () => ({
  getListening: () => h.listening,
  subscribeLevel: (cb: (l: number) => void) => {
    h.level = cb;
    return () => {
      h.level = null;
    };
  },
  subscribeListening: () => () => {},
}));

let frames: FrameRequestCallback[] = [];
let clock = 0;
const runFrame = () => {
  const due = frames;
  frames = [];
  clock += 16;
  for (const cb of due) cb(clock);
};

function stubWindow(motion: 'full' | 'reduced') {
  vi.stubGlobal('document', {
    visibilityState: 'visible',
    documentElement: { dataset: { motion } },
  });
  vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => {
    frames.push(cb);
    return frames.length;
  });
  vi.stubGlobal('cancelAnimationFrame', () => {
    frames = [];
  });
  vi.stubGlobal(
    'MutationObserver',
    class {
      observe() {}
      disconnect() {}
    },
  );
}

function bars() {
  return Array.from({ length: 5 }, () => ({ style: {} as Record<string, string> })) as unknown as HTMLElement[];
}

beforeEach(() => {
  vi.resetModules();
  frames = [];
  h.listening = { id: 'a', phase: 'listening' };
  h.level = null;
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('the listening bars loop', () => {
  it('Reduced, breathing: draws once and schedules no further frame', async () => {
    stubWindow('reduced');
    const { drawBars } = await import('./listeningBars');
    const stop = drawBars(bars());
    expect(frames).toHaveLength(1);
    runFrame();
    expect(frames).toHaveLength(0);
    stop();
  });

  it('Reduced: the first heard level starts the loop, and following keeps it running', async () => {
    stubWindow('reduced');
    const { drawBars } = await import('./listeningBars');
    const stop = drawBars(bars());
    runFrame();
    expect(frames).toHaveLength(0);
    h.level?.(0.3);
    expect(frames).toHaveLength(1);
    runFrame();
    expect(frames).toHaveLength(1);
    runFrame();
    expect(frames).toHaveLength(1);
    stop();
  });

  it('Full: keeps scheduling while breathing', async () => {
    stubWindow('full');
    const { drawBars } = await import('./listeningBars');
    const stop = drawBars(bars());
    runFrame();
    runFrame();
    expect(frames).toHaveLength(1);
    stop();
  });

  it('Full in the cheap look draws like Reduced, and leaving it starts the loop again', async () => {
    stubWindow('full');
    const lp = await import('../../utils/lowPower');
    const { drawBars } = await import('./listeningBars');
    lp.setLowPowerSource('performance', { quiet: true, still: false, flat: true });
    const stop = drawBars(bars());
    runFrame();
    expect(frames).toHaveLength(0);
    lp.setLowPowerSource('performance', { quiet: false, still: false, flat: false });
    expect(frames).toHaveLength(1);
    runFrame();
    expect(frames).toHaveLength(1);
    stop();
  });

  it('barsMove: Full always, Reduced only while following the level', async () => {
    stubWindow('full');
    const { barsMove } = await import('./listeningBars');
    expect(barsMove('breathing', 'full')).toBe(true);
    expect(barsMove('level', 'reduced')).toBe(true);
    expect(barsMove('breathing', 'reduced')).toBe(false);
    expect(barsMove('finding', 'reduced')).toBe(false);
    expect(barsMove('level', 'off')).toBe(false);
  });
});
