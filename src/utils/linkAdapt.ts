// Every player's side of the core's link governor (Rust `link_adapt`).
//
// When the quality is "Auto", Rust decides which rendition plays, on every
// platform and for every stream: the solo player and each MultiNook tile. A
// player only reports what it saw and follows the answer:
//
//   - every whole segment it downloads itself (bytes, time), on the
//     whole-segment path only. On the parts path it reads from the relay over
//     loopback, which says nothing about the link, so Rust measures it there
//     itself (and drops reports from any relay that serves from memory);
//   - stalls, once a second while frozen and once when they end;
//   - `playback-adapt` comes back naming the stream and the rendition now
//     playing, with a URL when the relay restarted and the player reloads.
import { invoke } from '@tauri-apps/api/core';

/** The solo player's stream id; MultiNook tiles use their cell ids. */
export const SOLO_STREAM_ID = 'solo';

export interface PlaybackAdapt {
  /** The stream it concerns: `solo`, or a MultiNook tile's id. */
  streamId?: string;
  /** The rendition now playing. */
  quality: string;
  auto: boolean;
  extraCushion: number;
  url?: string | null;
  reason: string;
}

/** Whether an event concerns the solo player (older cores sent no id). */
export function isSoloAdapt(event: PlaybackAdapt): boolean {
  return !event.streamId || event.streamId === SOLO_STREAM_ID;
}

/** What a player knows about the segment it downloaded. */
export interface SegmentFacts {
  /** The segment's discontinuity counter (hls.js `frag.cc`). */
  cc: number;
  /** How long the segment plays, in milliseconds. */
  durationMs: number;
}

function report(
  event: 'frag' | 'stall' | 'stall_end',
  bytes: number | null,
  ms: number,
  streamId: string | undefined,
  facts?: SegmentFacts,
): void {
  invoke('report_playback_health', {
    event,
    bytes,
    ms,
    streamId: streamId ?? null,
    cc: facts ? facts.cc : null,
    mediaMs: facts ? facts.durationMs : null,
  }).catch(() => {
    /* best effort: a lost report only delays a decision */
  });
}

/**
 * A whole segment downloaded from the network. The facts let Rust leave the
 * first segment of a new discontinuity run out and time what the rendition's
 * content really needs.
 */
export function reportSegment(bytes: number, ms: number, streamId?: string, facts?: SegmentFacts): void {
  if (bytes > 0 && ms > 0) report('frag', bytes, ms, streamId, facts);
}

/**
 * Stall bookkeeping for one player. `begin` on `waiting`, `end` on `playing`;
 * while it lasts the core hears about it once a second, so a stall that never
 * ends (a rendition the link cannot carry) still gets a decision.
 */
export function createStallReporter(streamId?: string) {
  let since: number | null = null;
  let timer: number | null = null;
  const stop = () => {
    if (timer !== null) {
      window.clearInterval(timer);
      timer = null;
    }
  };
  return {
    begin() {
      if (since !== null) return;
      since = performance.now();
      timer = window.setInterval(() => {
        if (since !== null) report('stall', null, performance.now() - since, streamId);
      }, 1000);
    },
    end() {
      stop();
      if (since === null) return;
      const ms = performance.now() - since;
      since = null;
      report('stall_end', null, ms, streamId);
    },
    /** Teardown, a pause or a seek: the freeze was not the link's doing. */
    cancel() {
      stop();
      since = null;
    },
  };
}
