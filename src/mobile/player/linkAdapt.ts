// The phone player's side of the core's link governor (Rust `link_adapt`).
//
// Rust decides which rendition plays on a link that changes under the phone
// (cellular, weak Wi-Fi), on the desktop's rules. The reports every player
// sends live in utils/linkAdapt; the player learns its own cushion, as the
// desktop's does (utils/llCushion):
//
//   - `playback-adapt` comes back with a URL when the relay restarted on
//     another rendition (a whole-segment switch is spliced into the running
//     playlist and needs nothing from the player).
//
// Module scope: there is one phone player (see the earning gate in
// useMobileHlsEngine for the same reasoning).
import { useAppStore } from '../../stores/AppStore';
import { Logger } from '../../utils/logger';
import { isSoloAdapt, type PlaybackAdapt } from '../../utils/linkAdapt';

export type { PlaybackAdapt } from '../../utils/linkAdapt';
export { createStallReporter, reportSegment } from '../../utils/linkAdapt';

export function applyPlaybackAdapt(event: PlaybackAdapt): void {
  if (!isSoloAdapt(event)) return;
  Logger.info(
    `[LinkAdapt] ${event.quality} (${event.reason}), extra cushion ${event.extraCushion.toFixed(1)}s` +
      (event.url ? ', reloading' : ''),
  );
  // Names the rendition "Auto" plays, and when the relay restarted on another
  // one, reloads on its URL exactly as a quality change does (never reviving a
  // stream already closed).
  useAppStore.getState().applyLinkAdapt(event);
}

