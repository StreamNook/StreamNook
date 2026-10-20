// The phone player's side of the core's link governor (Rust `link_adapt`).
//
// Rust decides which rendition plays and how far behind the live edge to sit
// on a link that changes under the phone (cellular, weak Wi-Fi). The reports
// every player sends live in utils/linkAdapt; what is phone-only is the
// cushion, which the phone player follows (the desktop learns its own):
//
//   - `playback-adapt` comes back with the extra cushion, and with a URL when
//     the relay restarted on another rendition (a whole-segment switch is
//     spliced into the running playlist and needs nothing from the player).
//
// Module scope: there is one phone player (see the earning gate in
// useMobileHlsEngine for the same reasoning).
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import { Logger } from '../../utils/logger';
import { isSoloAdapt, type PlaybackAdapt } from '../../utils/linkAdapt';

export type { PlaybackAdapt } from '../../utils/linkAdapt';
export { createStallReporter, reportSegment } from '../../utils/linkAdapt';

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
  if (!isSoloAdapt(event)) return;
  extraCushion = event.extraCushion;
  Logger.info(
    `[LinkAdapt] ${event.quality} (${event.reason}), extra cushion ${event.extraCushion.toFixed(1)}s` +
      (event.url ? ', reloading' : ''),
  );
  // Names the rendition "Auto" plays, and when the relay restarted on another
  // one, reloads on its URL exactly as a quality change does (never reviving a
  // stream already closed).
  useAppStore.getState().applyLinkAdapt(event);
}

