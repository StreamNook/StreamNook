// Cap the video ladder to what this screen can actually show.
//
// The phone was resolving Twitch's 2560x1440 source onto a display whose short
// edge is 1080, and rendering it into a portrait band about 1080x608. That is
// several times the pixels the screen can display, at roughly double the
// bitrate: decoder power, memory bandwidth, GPU scaling and radio time all spent
// on pixels that get thrown away before they reach the panel.
//
// The cap has to live in RUST, not in the player. StreamNook resolves to a
// SINGLE variant before hls.js ever sees a manifest, so `capLevelToPlayerSize`
// would be a no-op - there is no ladder left for the player to choose from.
//
// Only "best"/auto is capped. An explicitly chosen rendition is name-matched
// earlier in the resolver and wins outright, which is what makes the quality
// menu a genuine override.
import { invoke } from '@tauri-apps/api/core';
import { Logger } from '../utils/logger';

// The ladder Twitch actually publishes. The cap is snapped DOWN to one of these
// so a rounding artefact (an odd panel, a scaled DPR) can never sit just under a
// rung and knock the stream down a whole tier.
const RUNGS = [360, 480, 720, 1080, 1440, 2160];

function snapDown(px: number): number {
  let out = RUNGS[0];
  for (const r of RUNGS) if (px >= r) out = r;
  return out;
}

/**
 * The display's short edge in real pixels, or 0 if it cannot be determined.
 *
 * Prefers the native bridge, which reads `display.mode` - the mode Android is
 * CURRENTLY driving. That distinction matters: this phone ships a 1440x3168
 * panel that runs at 1080x2376 unless the user changes it, and `wm size` /
 * DisplayMetrics report the active mode while the panel spec says otherwise.
 *
 * The JS fallback (screen size x DPR) tracks the active mode correctly too, but
 * only the bridge is authoritative, and only the bridge is reliable at boot
 * before layout has settled.
 */
function displayShortEdgePx(): number {
  try {
    const b = (window as Window & { SNInsets?: { displayShortEdge?(): number } }).SNInsets;
    const native = b?.displayShortEdge?.();
    if (typeof native === 'number' && native > 0) return native;
  } catch {
    /* bridge absent */
  }
  try {
    const dpr = Math.max(1, window.devicePixelRatio || 1);
    const shortCss = Math.min(window.screen.width, window.screen.height);
    return Math.round(shortCss * dpr);
  } catch {
    return 0;
  }
}

// The cap last sent. A display change that lands on the same rung (a
// refresh-rate flip, a rotation) sends nothing.
let lastCap = 0;

/** The cap to send for this short edge, or null when there is none or it is unchanged. */
export function nextCap(px: number, last: number): number | null {
  if (px <= 0) return null;
  const cap = snapDown(px);
  return cap === last ? null : cap;
}

async function push(): Promise<void> {
  const px = displayShortEdgePx();
  const cap = nextCap(px, lastCap);
  if (cap === null) return;
  lastCap = cap;
  try {
    await invoke('set_max_video_height', { height: cap });
    Logger.info(`[videoCap] display short edge ${px}px -> capping auto quality at ${cap}p`);
  } catch (err) {
    // Non-fatal: an uncapped resolve is the old behaviour, not a broken one.
    lastCap = 0;
    Logger.warn('[videoCap] could not set the video height cap', err);
  }
}

/**
 * Install the cap. Safe to call once at mobile boot.
 *
 * Re-pushes on display change because an Android device can switch display mode
 * at runtime and a value sampled once at startup goes stale exactly when it
 * matters. A `resize` is NOT a usable signal for this: both of this panel's
 * modes are 360 CSS px wide, so a switch changes devicePixelRatio while
 * innerWidth stays put and no resize fires. MainActivity's DisplayListener
 * raises `sn:display-changed` instead.
 *
 * A new cap applies at the NEXT resolve. Nothing reloads an already-playing
 * stream, which is deliberate: a silent mid-stream reload is worse than one
 * stream finishing at the previous tier.
 */
export function installVideoCap(): () => void {
  void push();
  const onChange = () => void push();
  // Every return to the app tries again too: sn:display-changed now fires only
  // on a size change, so a boot push that failed would otherwise never retry.
  // An unchanged rung sends nothing (nextCap).
  const onVisible = () => {
    if (document.visibilityState === 'visible') void push();
  };
  window.addEventListener('sn:display-changed', onChange);
  document.addEventListener('visibilitychange', onVisible);
  return () => {
    window.removeEventListener('sn:display-changed', onChange);
    document.removeEventListener('visibilitychange', onVisible);
  };
}
