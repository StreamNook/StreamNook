import { afterEach, describe, expect, it, vi } from 'vitest';
import type Hls from 'hls.js';

const invoke = vi.fn((..._args: unknown[]) => Promise.resolve());
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
let tick: (() => void) | null = null;
vi.mock('./tileTicker', () => ({
  onTileTick: (fn: () => void) => {
    tick = fn;
    return () => {
      tick = null;
    };
  },
}));

const { alignTile, alignTiles, isSynced, registerSyncTile, syncDelayFor } = await import('./tileSync');

function fakeTile(opts: {
  latency: number;
  now: number;
  start: number;
  end: number;
  lowLatency: boolean;
  windowStart?: number;
}) {
  const video = {
    paused: false,
    readyState: 4,
    currentTime: opts.now,
    buffered: { length: 1, start: () => opts.start, end: () => opts.end },
  } as unknown as HTMLVideoElement;
  const details = { targetduration: 2, fragmentStart: opts.windowStart ?? -1000 };
  const hls = {
    latency: opts.latency,
    currentLevel: 0,
    levels: [{ details }],
    latestLevelDetails: details,
    targetLatency: 0,
  } as unknown as Hls;
  return { video, hls, details, lowLatency: opts.lowLatency, cushion: opts.lowLatency ? 3 : 8 };
}

const offs: Array<() => void> = [];
afterEach(() => {
  while (offs.length) offs.pop()!();
  invoke.mockClear();
});

describe('tileSync', () => {
  it('holds every tile at the least delay all of them can keep', () => {
    const ll = fakeTile({ latency: 3, now: 100, start: 90, end: 102, lowLatency: true });
    const seg = fakeTile({ latency: 10, now: 50, start: 45, end: 58, lowLatency: false });
    offs.push(registerSyncTile('ll', ll), registerSyncTile('seg', seg));
    alignTiles();
    expect(isSynced()).toBe(true);
    // The whole-segment tile settles a segment past its 8 s cushion.
    expect(syncDelayFor('ll')).toBe(10);
    expect(invoke).toHaveBeenCalledWith('set_multi_nook_sync_delay', { seconds: 10 });
    // hls.js's own target follows, so its live-edge resync lands there too.
    expect(ll.hls.targetLatency).toBe(10);
    // The low-latency tile steps back 7 s inside its back buffer.
    expect(ll.video.currentTime).toBeCloseTo(93);
    expect(seg.video.currentTime).toBe(50);
  });

  it('waits for the playlist window to reach back, then moves once', () => {
    const ll = fakeTile({ latency: 3, now: 100, start: 90, end: 102, lowLatency: true, windowStart: 96 });
    const seg = fakeTile({ latency: 10, now: 50, start: 45, end: 58, lowLatency: false });
    offs.push(registerSyncTile('ll', ll), registerSyncTile('seg', seg));
    alignTiles();
    expect(ll.video.currentTime).toBe(100);
    expect(tick).not.toBeNull();
    tick!();
    expect(ll.video.currentTime).toBe(100);
    // Two more segments listed: the window now starts before the target.
    ll.details.fragmentStart = 91;
    tick!();
    expect(ll.video.currentTime).toBeCloseTo(93);
    expect(tick).toBeNull();
  });

  it('moves forward within what is buffered', () => {
    const a = fakeTile({ latency: 10, now: 50, start: 45, end: 58, lowLatency: false });
    const b = fakeTile({ latency: 14, now: 50, start: 45, end: 60, lowLatency: false });
    offs.push(registerSyncTile('a', a), registerSyncTile('b', b));
    alignTiles();
    expect(b.video.currentTime).toBeCloseTo(54);
  });

  it('never jumps a low-latency tile forward unless LIVE was clicked', () => {
    const seg = fakeTile({ latency: 10, now: 50, start: 45, end: 58, lowLatency: false });
    const ll = fakeTile({ latency: 14, now: 100, start: 95, end: 108, lowLatency: true });
    offs.push(registerSyncTile('seg', seg), registerSyncTile('ll', ll));
    alignTiles();
    expect(ll.video.currentTime).toBe(100);
    alignTile('ll', true);
    expect(ll.video.currentTime).toBeCloseTo(104);
  });

  it('needs two playing tiles, and ends when the last tile leaves', () => {
    const only = fakeTile({ latency: 3, now: 10, start: 0, end: 12, lowLatency: true });
    const off = registerSyncTile('only', only);
    alignTiles();
    expect(isSynced()).toBe(false);
    const other = fakeTile({ latency: 10, now: 10, start: 0, end: 18, lowLatency: false });
    const off2 = registerSyncTile('other', other);
    alignTiles();
    expect(isSynced()).toBe(true);
    off();
    off2();
    expect(isSynced()).toBe(false);
    expect(syncDelayFor('only')).toBeNull();
    expect(invoke).toHaveBeenLastCalledWith('set_multi_nook_sync_delay', { seconds: null });
  });
});
