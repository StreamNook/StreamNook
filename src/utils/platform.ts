/**
 * Synchronous platform checks for chrome that must be right on the first paint.
 *
 * Deliberately user-agent based rather than `@tauri-apps/plugin-os`: that
 * plugin is async, and a title bar that learns its platform one tick late
 * renders the Windows control cluster and then yanks it away, which reads as a
 * flicker on every single launch. WKWebView reports `Macintosh` on macOS, so a
 * UA test is both correct and available before the first render.
 *
 * This is the desktop counterpart to the mobile shell's platform constants.
 */
export const IS_MAC =
  typeof navigator !== 'undefined' && /Macintosh|Mac OS X/.test(navigator.userAgent);

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
