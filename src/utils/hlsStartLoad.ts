import type Hls from 'hls.js';

/**
 * hls.js config for a player whose page decides when loading starts. With
 * `autoStartLoad` on, hls.js restarts loading by itself whenever its media
 * source opens or a media error is recovered, which undoes the stop a paused
 * player made (see `createPausePark`). With it off, loading runs only after a
 * `startLoad` call from the page.
 */
export const HLS_START_CONFIG = { autoStartLoad: false } as const;

/** A pause park, as far as the start decision needs it. */
export type LoadPark = { readonly loadStopped: boolean } | null | undefined;

/**
 * Start loading once the manifest is parsed, at the position the config names,
 * unless the player's park already stopped its loading on purpose. True when
 * loading was started.
 */
export function startLoadOnManifest(hls: Pick<Hls, 'startLoad' | 'config'>, park: LoadPark): boolean {
  if (park?.loadStopped) return false;
  hls.startLoad(hls.config.startPosition);
  return true;
}

/**
 * After `recoverMediaError`: start loading again when the recovery left it
 * stopped (a fatal error stops loading first, and hls.js restarts only a load
 * the page had started), unless the park stopped it on purpose. True when
 * loading was started.
 */
export function resumeLoadAfterRecover(
  hls: Pick<Hls, 'startLoad' | 'config' | 'loadingEnabled'>,
  park: LoadPark,
): boolean {
  if (park?.loadStopped || hls.loadingEnabled) return false;
  hls.startLoad(hls.config.startPosition);
  return true;
}
