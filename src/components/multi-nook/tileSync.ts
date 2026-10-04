// Lines MultiNook tiles up at one shared delay behind live.
//
// Every tile otherwise rides its own distance from its own live edge: 3 s on
// the low-latency path, a segment past an 8 s cushion on the whole-segment
// path. Two streams of the same event therefore sit seconds apart even when
// both are healthy. Resync picks the smallest delay every playing tile can
// hold safely, moves each tile there inside what it has already buffered (no
// restart, no rebuffer), and leaves each tile's latency governor holding it
// there until MultiNook closes.
//
// Three things have to agree for a tile to STAY at the shared delay:
//   * its governor holds it (the governor reads `syncDelayFor`);
//   * hls.js's own target is the same delay, so any live-edge resync hls.js
//     does on its own lands there and not at the tile's cushion;
//   * its playlist reaches that far back. hls.js snaps a playhead that sits
//     before the first listed segment straight back to live as soon as it
//     buffers, and a low-latency tile lists only ~8 s, so Rust widens the
//     tile windows for the sync (`set_multi_nook_sync_delay`). The window
//     grows as segments arrive, so a tile that has to move back waits until
//     both its window and its back buffer cover the move, then moves once.
//
// The delay is `hls.latency`: distance from the tile's playlist edge,
// extrapolated by the playlist's age, the one measure every path reports the
// same way. It cannot see a delay the streamer adds before Twitch (an OBS
// stream delay, a slow encoder); no client measure can.

import { invoke } from '@tauri-apps/api/core';
import type Hls from 'hls.js';
import { Logger } from '../../utils/logger';
import { onTileTick } from './tileTicker';

export interface SyncTile {
  hls: Hls;
  video: HTMLVideoElement;
  /** Rides the low-latency origin: forward seeks there freeze playback, so
   *  only the governor may pull it toward live. */
  lowLatency: boolean;
  /** The tile's own cushion (its `liveSyncDuration`). */
  cushion: number;
  /** Tell the tile its playhead moved, so its LIVE readout re-bases. */
  onSeek?: (deltaSecs: number) => void;
}

const tiles = new Map<string, SyncTile>();
let target: number | null = null;

/** Seconds of buffer kept ahead of the playhead after a forward move. */
const FORWARD_MARGIN = 1.5;
/** Seconds kept inside the playlist's first segment after a backward move. */
const WINDOW_MARGIN = 1;
/** Closer than this is already in sync: no seek. */
const SEEK_THRESHOLD = 0.25;
/** How long a tile waits for its window and buffer before it moves as far as
 *  it can. The window grows a segment (2 s) at a time. */
const PENDING_MS = 30_000;

/** Tiles still waiting to move, and when they gave up waiting. */
const pending = new Map<string, number>();
let stopTick: (() => void) | null = null;

function latencyOf(t: SyncTile): number | null {
  const l = t.hls.latency;
  return Number.isFinite(l) && l > 0 ? l : null;
}

/** The least delay this tile holds without stalling. A whole-segment tile
 *  settles a segment past its cushion (its buffer fills a segment at a time),
 *  so that is where it already plays; a low-latency tile holds its cushion. */
function minDelay(t: SyncTile): number {
  if (t.lowLatency) return t.cushion;
  const level = t.hls.levels?.[t.hls.currentLevel];
  const seg = level?.details?.targetduration;
  return t.cushion + (typeof seg === 'number' && seg > 0 ? seg : 2);
}

function playing(t: SyncTile): boolean {
  return !t.video.paused && t.video.readyState >= 2 && latencyOf(t) != null;
}

type Move = 'done' | 'wait';

/**
 * Move one tile to `delay`. The move happens in one seek or not at all, so
 * a tile never visibly jumps twice: when its buffer or playlist window does
 * not reach yet it answers 'wait'. `force` takes whatever part of the move
 * is possible (a LIVE click, or a wait that ran out).
 */
function moveTo(id: string, t: SyncTile, delay: number, force = false, userAction = false): Move {
  const latency = latencyOf(t);
  if (latency == null) return 'wait';
  const delta = latency - delay; // > 0: too far behind live
  if (Math.abs(delta) < SEEK_THRESHOLD) return 'done';
  const video = t.video;
  const now = video.currentTime;
  const b = video.buffered;
  let start = now;
  let end = now;
  for (let i = 0; i < b.length; i++) {
    if (b.start(i) <= now + 0.05 && b.end(i) >= now - 0.05) {
      start = b.start(i);
      end = b.end(i);
      break;
    }
  }
  const want = now + delta;
  let reach: number;
  if (delta > 0) {
    // A low-latency tile is never jumped forward, except by its LIVE button
    // (as LIVE always has); its governor closes the gap instead.
    if (t.lowLatency && !userAction) return 'done';
    reach = Math.min(want, end - FORWARD_MARGIN);
  } else {
    const details = t.hls.latestLevelDetails;
    const windowStart = details ? details.fragmentStart + WINDOW_MARGIN : -Infinity;
    reach = Math.max(want, start + 0.1, windowStart);
  }
  const full = Math.abs(reach - want) < SEEK_THRESHOLD;
  if (!full && !force) return 'wait';
  if (Math.abs(reach - now) < SEEK_THRESHOLD || (delta > 0 ? reach < now : reach > now)) return 'done';
  video.currentTime = reach;
  t.onSeek?.(reach - now);
  Logger.debug(`[MultiNook-sync] ${id}: ${latency.toFixed(1)}s -> ${delay.toFixed(1)}s (moved ${(reach - now).toFixed(1)}s)`);
  return 'done';
}

function settlePending(): void {
  const now = performance.now();
  for (const [id, until] of pending) {
    const t = tiles.get(id);
    const delay = syncDelayFor(id);
    if (!t || delay == null) {
      pending.delete(id);
      continue;
    }
    if (!playing(t)) continue;
    if (moveTo(id, t, delay, now > until) === 'done') pending.delete(id);
  }
  if (pending.size === 0 && stopTick) {
    stopTick();
    stopTick = null;
  }
}

function settle(id: string, t: SyncTile, delay: number, userAction = false): void {
  // hls.js resyncs to its own target when the playhead strays; make that the
  // shared delay so it never undoes the line-up.
  t.hls.targetLatency = delay;
  if (moveTo(id, t, delay, userAction, userAction) === 'done') {
    pending.delete(id);
    return;
  }
  pending.set(id, performance.now() + PENDING_MS);
  if (!stopTick) stopTick = onTileTick(settlePending);
}

function publishDelay(delay: number | null): void {
  void invoke('set_multi_nook_sync_delay', { seconds: delay }).catch((e: unknown) =>
    Logger.warn('[MultiNook-sync] could not widen tile windows:', e),
  );
}

/** The delay a tile's governor should hold, or null outside a sync. A tile
 *  that cannot go as low as the shared delay holds its own minimum. */
export function syncDelayFor(id: string): number | null {
  if (target == null) return null;
  const t = tiles.get(id);
  return t ? Math.max(target, minDelay(t)) : target;
}

/** Whether a Resync is holding the tiles together. */
export function isSynced(): boolean {
  return target != null;
}

/** Register a playing tile. Returns the unregister; the sync ends with the
 *  last tile. */
export function registerSyncTile(id: string, tile: SyncTile): () => void {
  tiles.set(id, tile);
  if (target != null) tile.hls.targetLatency = Math.max(target, minDelay(tile));
  return () => {
    if (tiles.get(id) !== tile) return;
    tiles.delete(id);
    pending.delete(id);
    if (tiles.size === 0 && target != null) {
      target = null;
      publishDelay(null);
    }
  };
}

/** Move one tile to the shared delay, if a sync is on. `userAction` is a
 *  click on the tile's LIVE control. */
export function alignTile(id: string, userAction = false): void {
  const t = tiles.get(id);
  const delay = syncDelayFor(id);
  if (t && delay != null && playing(t)) settle(id, t, delay, userAction);
}

/** Resync: every playing tile to the least delay all of them can hold. */
export function alignTiles(): void {
  const live = [...tiles.entries()].filter(([, t]) => playing(t));
  if (live.length < 2) return;
  target = Math.round(Math.max(...live.map(([, t]) => minDelay(t))) * 10) / 10;
  publishDelay(target);
  Logger.info(`[MultiNook-sync] holding ${live.length} tiles at ${target}s behind live`);
  for (const [id, t] of live) settle(id, t, Math.max(target, minDelay(t)));
}
