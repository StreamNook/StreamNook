// FIRST import on purpose: turns off React 19's dev-only per-element
// instrumentation before react-dom evaluates (see the file for numbers).
import './devReactInstrumentation';
import React, { lazy, Suspense } from 'react';
import ReactDOM from 'react-dom/client';
import { MotionScope } from './components/MotionScope.tsx';
// Side-effect import: starts the settings IPC now, before the lazy route
// imports below, so it overlaps chunk loading.
import './bootPreload';

// Route components are lazy so each window only downloads/parses the code it
// actually renders. The MultiChat / profile / plugin popouts no longer pull in
// App's whole tree (video player + hls.js/plyr, browse, settings) — a real
// footprint + startup cut for the chat-only popout.
const App = lazy(() => import('./App.tsx'));
const MobileApp = lazy(() => import('./mobile/MobileApp.tsx'));
const ProfileCardPage = lazy(() => import('./pages/ProfileCardPage.tsx'));
const MultiChatWindow = lazy(() => import('./components/multichat/MultiChatWindow.tsx'));
const ChatOverlayWindow = lazy(() => import('./components/multichat/ChatOverlayWindow'));
const PluginWindowHost = lazy(() => import('./plugins-ui/PluginWindowHost.tsx'));
// Popout-window and tray plumbing. These used to be unconditional side-effect
// imports, so they registered at module load on Android too, where there is no
// tray and WebviewWindow.create() throws. Desktop-only now; the microtask delay
// is irrelevant because both are driven by later user interaction.
if (!IS_MOBILE) {
  // registers `window.openMultiChatWindow` for popout spawning
  import('./utils/multichatWindow');
  // listens for the tray's "Open MultiChat" menu event
  import('./utils/multichatTrayBridge');
}
// Fraunces (variable serif). The upright axis backs the "Serif" choice in
// Theme > Font, so its @font-face must exist at boot for users who chose it
// (the woff2 itself only downloads when rendered). The italic axis is only
// used by the tier-badge rank number and rides StreamNookBadge.tsx instead.
import '@fontsource-variable/fraunces';
import './styles/globals.css';
// Light treatment for the Prism theme. Separate from globals.css so the effect
// is one self-contained sheet, and loaded after it so its selectors win.
import './styles/theme-prism.css';
// Mobile layout layer. Every rule is scoped behind html[data-mobile="true"],
// which is set just below, so importing it on desktop is inert.
import './styles/mobile.css';
import { initLogCapture } from './services/logService';
import { IS_MOBILE, isPortrait, onOrientationChange } from './utils/platform';

// Drive the mobile CSS off the document element. Orientation is tracked here
// rather than with a CSS media query because the layout branch also needs it in
// JS (the player switches between a fixed 16:9 band and full-bleed).
if (IS_MOBILE) {
  const root = document.documentElement;
  root.dataset.mobile = 'true';
  const applyOrientation = () => {
    root.dataset.orientation = isPortrait() ? 'portrait' : 'landscape';
  };
  applyOrientation();
  onOrientationChange(applyOrientation);
  // Android back-button chain for the in-place shell (MainActivity calls
  // window.__SN_BACK__). MobileApp overrides this with navStore on mount.
  void import('./mobile/inPlaceBack').then((m) => m.installInPlaceBackHandler());
  // Pull the native WindowInsets into --sn-inset-* CSS vars. The push from
  // MainActivity fires on inset CHANGES, which a fresh page load missed, and
  // env(safe-area-inset-*) reads 0 in this WebView, so without this pull the
  // UI draws under the status bar and camera cutout on every boot.
  void import('./mobile/nativeInsets').then((m) => m.applyNativeInsetsOnce());
  // Keep the status/navigation bar icon colour on the StreamNook theme rather
  // than on the phone's night-mode setting, which is what the platform would
  // otherwise guess from. Safe to land after the first theme apply: it reads the
  // live palette on install as well as listening for changes.
  void import('./mobile/systemBars').then((m) => m.installSystemBarAppearance());
}

import { Logger } from './utils/logger';
// Initialize log capture early to capture all console messages
initLogCapture();
Logger.debug('[App] StreamNook starting...');

// Remove Plyr's localStorage - we manage player settings via Tauri backend
// Plyr has built-in localStorage persistence that conflicts with our settings management
localStorage.removeItem('plyr');

// The old release-list cache. Rust keeps that list on disk now, so this copy
// is read by nothing.
localStorage.removeItem('streamnook_whatsnew_cache_v2');

// Route based on URL hash. Profile-card windows, the StreamNook MultiChat
// popout, and ui-plugin popout windows share the same bundle as the main App;
// main.tsx picks the root component to render.
const hash = window.location.hash;
const isProfileCard = hash.startsWith('#/profile');
const isMultiChat = hash.startsWith('#/multichat');
const isChatOverlay = hash.startsWith('#/chat-overlay');
const isPluginWindow = hash.startsWith('#/plugin/');

// The dedicated mobile shell (src/mobile/: bottom tabs, sheets, touch player,
// drill-in settings) is the mobile DEFAULT. The in-place adapted App
// (data-mobile CSS + MobileNav) remains reachable as an escape hatch: set
// localStorage['sn-legacy-shell'] = '1' on a device build to fall back.
const useNextMobileShell = IS_MOBILE && localStorage.getItem('sn-legacy-shell') !== '1';

// Create the React root ONCE per container. The lazy route imports above can make
// React Fast Refresh re-execute this module instead of full-reloading, and a second
// createRoot() on the same #root mounts a competing React tree — which manifests as
// the "createRoot() on a container that has already been passed" warning AND erratic
// freezes (two roots fighting over the same DOM, e.g. a clip modal locking up).
// Caching the root on the container makes re-execution a re-render, not a new root.
const container = document.getElementById('root') as HTMLElement & {
  __snRoot?: ReactDOM.Root;
};
// Dev-only console hooks. `withGlobalTauri` is deliberately off, so devtools has
// no way to reach a Tauri command; this exposes the handful worth poking at by
// hand rather than opening the whole API surface to any script in the window.
// Stripped from production builds by the DEV guard.
if (import.meta.env.DEV) {
  // React 19.2 development builds emit a performance.measure() entry for
  // every render, commit and effect (the DevTools performance tracks), and
  // the User Timing buffer keeps them for the page's lifetime: 12,500
  // entries after one minute of busy chat, about 570 MB of renderer memory
  // after ninety seconds, all released by clearMeasures() (measured
  // 2026-09-05). Production builds emit none. Drain the buffer so dev soak
  // numbers mean something; PerformanceObserver subscribers still receive
  // every entry, so profiling is unaffected. Set window.__snKeepMeasures =
  // true to inspect the buffer directly. Since devReactInstrumentation.ts the
  // tracks are off by default (localStorage 'sn-react-devtracks' = '1' turns
  // them back on), so this drain only has work when a profiling session
  // opted in.
  window.setInterval(() => {
    if ((window as unknown as { __snKeepMeasures?: boolean }).__snKeepMeasures) return;
    performance.clearMeasures();
    performance.clearMarks();
  }, 15_000);
  // React devtools bridge. This used to live in index.html gated on hostname,
  // but tauri.localhost is the PRODUCTION origin on Windows, so shipped builds
  // were loading a script from a local port any process could bind. The DEV
  // guard strips it from release bundles entirely.
  const devtools = document.createElement('script');
  devtools.src = 'http://localhost:8097';
  document.head.appendChild(devtools);
  void import('@tauri-apps/api/core').then(({ invoke }) => {
    (window as unknown as Record<string, unknown>).sn = {
      /** One SABR round trip for a YouTube video id: mints a PO token, asks for
       *  media, and reports what came back. Watch the Rust log for the detail. */
      sabrProbe: (videoId: string) => invoke('youtube_sabr_probe', { videoId }),
    };
    // eslint-disable-next-line no-console
    console.info('[dev] window.sn ready: sn.sabrProbe("<videoId>")');
  });
}

// React 19 no longer rethrows render errors. Caught ones are logged by React
// itself and uncaught ones go to window.reportError, which nothing in this
// app listens to, so without these handlers an uncaught render error would
// reach the Rust log only as a bare console line with no component stack.
// ErrorBoundary already writes the user-facing line for caught errors, so
// that path stays quiet here.
const rootOptions: ReactDOM.RootOptions = {
  onUncaughtError: (error, info) => {
    Logger.error('[React] Uncaught render error:', error);
    Logger.error('[React] Component stack:', info.componentStack);
  },
  onCaughtError: (error) => {
    Logger.debug('[React] Error caught by a boundary:', error);
  },
  onRecoverableError: (error, info) => {
    Logger.warn('[React] Recovered from render error:', error);
    Logger.warn('[React] Component stack:', info.componentStack);
  },
};
const root = container.__snRoot ?? (container.__snRoot = ReactDOM.createRoot(container, rootOptions));

// Dev-only: expose the app store for CDP-driven test recipes (scratchpad
// cdp.mjs). Dynamic import keeps AppStore out of the entry chunk and the
// DEV guard strips it from production.
if (import.meta.env.DEV) {
  void import('./stores/AppStore').then((m) => {
    (window as unknown as { __snStore?: unknown }).__snStore = m.useAppStore;
  });
}
root.render(
  <React.StrictMode>
    <MotionScope>
      <Suspense fallback={null}>
        {isChatOverlay ? <ChatOverlayWindow /> : isMultiChat ? <MultiChatWindow /> : isPluginWindow ? <PluginWindowHost /> : isProfileCard ? <ProfileCardPage /> : useNextMobileShell ? <MobileApp /> : <App />}
      </Suspense>
    </MotionScope>
  </React.StrictMode>,
);
