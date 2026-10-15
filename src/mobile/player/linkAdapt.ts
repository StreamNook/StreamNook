// The phone player's side of the core's link governor (Rust `link_adapt`).
//
// Rust decides which rendition plays and how far behind the live edge to sit
// on a link that changes under the phone (cellular, weak Wi-Fi). The player
// only reports what it saw and follows the answer:
//
//   - every whole segment it downloads itself (bytes, time), on the
//     whole-segment path only; on the parts path it reads from the relay over
//     loopback and the origin measures the link instead;
//   - stalls, once a second while frozen and once when they end;
//   - `playback-adapt` comes back with the extra cushion, and with a URL when
//     the relay restarted on another rendition (a whole-segment switch is
//     spliced into the running playlist and needs nothing from the player).
//
// Module scope: there is one phone player (see the earning gate in
// useMobileHlsEngine for the same reasoning).
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import { Logger } from '../../utils/logger';

export interface PlaybackAdapt {
  quality: string;
  auto: boolean;
  extraCushion: number;
  url?: string | null;
  reason: string;
}

let extraCushion = 0;

/** Extra seconds behind the live edge the core wants kept right now. */
export function linkCushion(): number {
  return extraCushion;
}

/** Read once when a player is built; later moves arrive as `playback-adapt`. */
export async function refreshLinkCushion(): Promise<number> {
  try {
    extraCushion = await invoke<number>('get_live_edge_cushion');
  } catch {
    /* no session, or an older core */
  }
  return extraCushion;
}

export function applyPlaybackAdapt(event: PlaybackAdapt): void {
  extraCushion = event.extraCushion;
  Logger.info(
    `[LinkAdapt] ${event.quality} (${event.reason}), extra cushion ${event.extraCushion.toFixed(1)}s` +
      (event.url ? ', reloading' : ''),
  );
  // The relay restarted on another rendition: reload on its URL, exactly as a
  // quality change does. A stream already closed must not be revived.
  if (event.url && useAppStore.getState().streamUrl) {
    useAppStore.setState({ streamUrl: event.url });
  }
}

function report(event: 'frag' | 'stall' | 'stall_end', bytes: number | null, ms: number): void {
  invoke('report_playback_health', { event, bytes, ms }).catch(() => {
    /* best effort: a lost report only delays a decision */
  });
}

/** A whole segment downloaded from the network. */
export function reportSegment(bytes: number, ms: number): void {
  if (bytes > 0 && ms > 0) report('frag', bytes, ms);
}

/**
 * Stall bookkeeping for one player. `begin` on `waiting`, `end` on `playing`;
 * while it lasts the core hears about it once a second, so a stall that never
 * ends (a rendition the link cannot carry) still gets a decision.
 */
export function createStallReporter() {
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
        if (since !== null) report('stall', null, performance.now() - since);
      }, 1000);
    },
    end() {
      stop();
      if (since === null) return;
      const ms = performance.now() - since;
      since = null;
      report('stall_end', null, ms);
    },
    /** Teardown or a pause: the freeze was not the link's doing. */
    cancel() {
      stop();
      since = null;
    },
  };
}
