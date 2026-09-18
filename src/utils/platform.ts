// Runtime platform detection for the mobile port.
//
// This deliberately reads the user agent rather than @tauri-apps/plugin-os.
// The plugin would work, but it is an extra dependency AND an extra entry in
// capabilities/*.json (platform() is denied by the ACL without `os:default`),
// and it is async on some paths. The Android System WebView always reports
// "Android" in its UA, so this is both simpler and synchronous, which matters
// because the layout branch has to be correct on the very first render.

const ua = typeof navigator === 'undefined' ? '' : navigator.userAgent;

/** True inside the Android (and later iOS) builds, false on every desktop. */
export const IS_MOBILE = /android|iphone|ipad|ipod/i.test(ua);

export const IS_ANDROID = /android/i.test(ua);

/**
 * Synchronous macOS check for chrome that must be right on the first paint.
 *
 * Same reasoning as IS_MOBILE above: `@tauri-apps/plugin-os` is async, and a
 * title bar that learns its platform one tick late renders the Windows control
 * cluster and then yanks it away, which reads as a flicker on every single
 * launch. WKWebView reports `Macintosh` on macOS, so a UA test is both correct
 * and available before the first render. Gated off mobile so an iPad UA that
 * spells `Mac OS X` never counts as a desktop Mac.
 */
export const IS_MAC = !IS_MOBILE && /Macintosh|Mac OS X/.test(ua);

/**
 * Publish the platform to CSS, for the handful of rules that genuinely differ
 * rather than merely looking different. Set at module load, which runs during
 * the initial import graph and therefore before the first paint, for the same
 * reason IS_MAC is a synchronous UA test rather than an async plugin call.
 */
if (typeof document !== 'undefined') {
  document.documentElement.dataset.platform = IS_MAC ? 'mac' : IS_MOBILE ? 'mobile' : 'desktop';
}

/**
 * Width reserved at the leading edge of the title bar for macOS traffic
 * lights. They are drawn by AppKit (the window is decorated with an overlay
 * title bar), so the app must simply not put anything under them.
 *
 * Derived from where AppKit actually puts them: `trafficLightPosition.x` is 20
 * (the macOS standard inset), the three buttons are spaced ~20px apart, and the
 * last one is ~14px wide, so the group ends near x=74. 78 was tried first and
 * still clipped the Penrose logo / About button, so this leaves real breathing
 * room rather than a hairline. 92 also matches where native apps start their
 * own leading content (Safari, Finder), so it reads as deliberate.
 *
 * Keep in step with `trafficLightPosition` in `tauri.macos.conf.json`;
 * `tests/macos_window_parity.rs` documents the pairing.
 */
export const MAC_TRAFFIC_LIGHT_INSET_PX = 92;

/**
 * Portrait is the orientation the desktop layout cannot survive: it is a flex
 * row (sidebar + video + chat), so a docked chat panel squeezes the video to
 * nothing. Landscape is close enough to a small desktop window to reuse.
 */
export function isPortrait(): boolean {
  if (typeof window === 'undefined') return false;
  return window.innerHeight >= window.innerWidth;
}

/** Subscribe to orientation flips. Returns an unsubscribe. */
export function onOrientationChange(fn: () => void): () => void {
  if (typeof window === 'undefined') return () => {};
  window.addEventListener('resize', fn);
  window.addEventListener('orientationchange', fn);
  return () => {
    window.removeEventListener('resize', fn);
    window.removeEventListener('orientationchange', fn);
  };
}
