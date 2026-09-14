import { useEffect, useState, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Lock, X } from 'lucide-react';
import { Logger } from '../utils/logger';
import { useAppStore } from '../stores/AppStore';

// In-app Twitch overlay (login, drops sign-in, subscribe). React owns the chrome
// (the read-only address bar, and for subscribe the centered panel); the Twitch
// page itself is a single native child webview, because Twitch can't be iframed.
//
// Geometry is COMPUTED from the window size and shared constants (the same
// coordinates the old overlay hardcoded), not measured from the DOM, so the native
// webview always lands where the chrome expects it. React drives the webview's
// bounds (on real resizes only) and its visibility (hidden while minimized), so
// nothing runs in a native window-event callback. That callback resizing layered
// child webviews across a minimize was the Windows 10 alt-tab freeze.

type OverlayMode = 'fullbody' | 'panel';

interface OverlayState {
  label: string;
  url: string;
  mode: OverlayMode;
  /** Which persistent web profile backs it. Absent = the active Twitch
   *  account's, which is what every Twitch caller wants. */
  profile?: string;
}

// Matches the React TitleBar height (h-[40px]); the overlay sits just below it so
// the window controls and drag region stay usable.
const TITLE_BAR_HEIGHT = 40;
const URL_BAR_HEIGHT = 38;
const PANEL_MAX_W = 820;
const PANEL_MAX_H = 900;
const PANEL_MARGIN = 48;
// The native webview can't clip to the panel's rounded corners, so hold it just
// above the bottom; the strip below it is painted in Twitch's page background so
// the rounded edge blends into the live page with no mismatched band.
const PANEL_BODY_BOTTOM_INSET = 14;
// The panel's 1px border. The webview is inset by it on the left/right so the border
// shows around the full perimeter instead of being covered along the webview's sides.
const PANEL_BORDER = 1;
// Twitch's dark-theme purchase-bar background (--color-background-alt-2, rgb(38,38,44)),
// measured from the live subscribe page. That bar sits at the bottom of the webview, so
// the chin strip below it uses this color and the rounded bottom edge blends in.
const TWITCH_PAGE_BG = '#26262c';

interface Layout {
  // React chrome box (title-bar-relative window coords).
  boxLeft: number;
  boxTop: number;
  boxW: number;
  boxH: number;
  // Native Twitch webview rect (the chrome's body, below the address bar).
  wvX: number;
  wvY: number;
  wvW: number;
  wvH: number;
}

function computeLayout(mode: OverlayMode): Layout {
  const winW = window.innerWidth;
  const winH = window.innerHeight;
  if (mode === 'panel') {
    const boxW = Math.max(320, Math.min(PANEL_MAX_W, winW - PANEL_MARGIN));
    const boxH = Math.max(320, Math.min(PANEL_MAX_H, winH - TITLE_BAR_HEIGHT - PANEL_MARGIN));
    const boxLeft = Math.round((winW - boxW) / 2);
    const boxTop = Math.round(TITLE_BAR_HEIGHT + (winH - TITLE_BAR_HEIGHT - boxH) / 2);
    return {
      boxLeft,
      boxTop,
      boxW,
      boxH,
      wvX: boxLeft + PANEL_BORDER,
      wvY: boxTop + URL_BAR_HEIGHT,
      wvW: boxW - PANEL_BORDER * 2,
      wvH: Math.max(1, boxH - URL_BAR_HEIGHT - PANEL_BODY_BOTTOM_INSET),
    };
  }
  // fullbody (login / drops): fills the app body below the title bar.
  return {
    boxLeft: 0,
    boxTop: TITLE_BAR_HEIGHT,
    boxW: winW,
    boxH: winH - TITLE_BAR_HEIGHT,
    wvX: 0,
    wvY: TITLE_BAR_HEIGHT + URL_BAR_HEIGHT,
    wvW: winW,
    wvH: Math.max(1, winH - TITLE_BAR_HEIGHT - URL_BAR_HEIGHT),
  };
}

export default function TwitchOverlay() {
  const [overlay, setOverlay] = useState<OverlayState | null>(null);
  const [displayUrl, setDisplayUrl] = useState('');
  // Bumped on window resize so the layout recomputes and the webview re-syncs.
  const [tick, setTick] = useState(0);
  const mountedLabelRef = useRef<string | null>(null);
  const lastKeyRef = useRef('');

  // Backend events: open a new overlay, live URL updates, and dismissal.
  useEffect(() => {
    const uns: Array<() => void> = [];
    listen<{ label: string; url: string; mode: OverlayMode; profile?: string }>('twitch-overlay-open', (e) => {
      setOverlay((cur) => {
        if (cur && cur.label !== e.payload.label) {
          invoke('close_login_overlay', { label: cur.label }).catch(() => {});
        }
        return { label: e.payload.label, url: e.payload.url, mode: e.payload.mode, profile: e.payload.profile };
      });
      setDisplayUrl(e.payload.url);
    }).then((u) => uns.push(u));

    listen<{ label: string; url: string }>('twitch-overlay-url', (e) => {
      setOverlay((cur) => {
        if (cur && cur.label === e.payload.label) setDisplayUrl(e.payload.url);
        return cur;
      });
    }).then((u) => uns.push(u));

    listen<{ label: string }>('twitch-overlay-close', (e) => {
      setOverlay((cur) => (cur && cur.label === e.payload.label ? null : cur));
    }).then((u) => uns.push(u));

    return () => uns.forEach((u) => u());
  }, []);

  // Recompute on real window resizes only.
  useEffect(() => {
    const onResize = () => setTick((t) => t + 1);
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, []);

  // The overlay is a separate top-level window, so it has to follow the main window
  // when it moves or resizes. Bumping tick re-runs the sync effect below.
  useEffect(() => {
    const w = getCurrentWindow();
    const uns: Array<() => void> = [];
    w.onMoved(() => setTick((t) => t + 1)).then((u) => uns.push(u));
    w.onResized(() => setTick((t) => t + 1)).then((u) => uns.push(u));
    return () => uns.forEach((u) => u());
  }, []);

  // Hide the native webview while the window is minimized, show on restore. This is
  // Microsoft's prescribed WebView2 minimize/restore fix (the Windows 10 freeze).
  useEffect(() => {
    if (!overlay) return;
    const label = overlay.label;
    const onVis = () => {
      invoke('set_twitch_overlay_visible', { label, visible: !document.hidden }).catch(() => {});
    };
    document.addEventListener('visibilitychange', onVis);
    return () => document.removeEventListener('visibilitychange', onVis);
  }, [overlay]);

  // Open the overlay's Twitch window and keep it tracking the React chrome. It's a
  // separate top-level (owned) window, so it's placed in SCREEN coords: the main
  // window's client-area origin plus the body rect React measured. Deduped by rect key
  // so an unchanged layout never repositions it.
  useEffect(() => {
    if (!overlay) {
      mountedLabelRef.current = null;
      lastKeyRef.current = '';
      return;
    }
    let cancelled = false;
    (async () => {
      const l = computeLayout(overlay.mode);
      let originX = 0;
      let originY = 0;
      try {
        const w = getCurrentWindow();
        const scale = await w.scaleFactor();
        const inner = await w.innerPosition(); // physical px of client-area top-left
        originX = inner.x / scale;
        originY = inner.y / scale;
      } catch {
        // No window origin (non-Tauri/dev): fall back to window-relative coords.
      }
      if (cancelled) return;
      const x = originX + l.wvX;
      const y = originY + l.wvY;
      const args = { x, y, width: l.wvW, height: l.wvH };
      const key = `${Math.round(x)}|${Math.round(y)}|${l.wvW}|${l.wvH}`;
      if (mountedLabelRef.current !== overlay.label) {
        mountedLabelRef.current = overlay.label;
        lastKeyRef.current = key;
        invoke('mount_twitch_overlay', { label: overlay.label, url: overlay.url, profile: overlay.profile ?? null, ...args }).catch((e) => {
          Logger.warn('[TwitchOverlay] open failed:', e);
          useAppStore.getState().addToast(`Login window failed to open: ${String(e)}`, 'error');
        });
      } else if (key !== lastKeyRef.current) {
        lastKeyRef.current = key;
        invoke('set_twitch_overlay_bounds', { label: overlay.label, ...args }).catch(() => {});
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [overlay, tick]);

  const close = useCallback(() => {
    setOverlay((cur) => {
      if (cur) invoke('close_login_overlay', { label: cur.label }).catch(() => {});
      return cur;
    });
  }, []);

  // Esc dismisses (the Twitch webview itself also handles Esc when it has focus).
  useEffect(() => {
    if (!overlay) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [overlay, close]);

  if (!overlay) return null;
  const l = computeLayout(overlay.mode);

  // Address bar styled like a real browser omnibox: neutral gray padlock, the
  // domain emphasized, the scheme dropped and the path dimmed, in the system UI
  // font (not monospace), so it reads as a trustworthy browser chrome.
  const renderBar = (withClose: boolean) => {
    let host = displayUrl;
    let rest = '';
    try {
      const u = new URL(displayUrl);
      host = u.hostname;
      rest = u.pathname + u.search + u.hash;
    } catch {
      // Non-absolute URL: show it verbatim.
    }
    return (
      <div
        className="flex items-center gap-2 px-3 bg-[#18181b] border-b border-white/5 select-none"
        style={{ height: URL_BAR_HEIGHT, fontFamily: 'system-ui, -apple-system, "Segoe UI", Roboto, sans-serif' }}
      >
        <Lock size={13} className="shrink-0 text-[#3fb950]" />
        <span className="flex-1 truncate select-text text-[12.5px] leading-none">
          <span className="text-white/90">{host}</span>
          {rest && <span className="text-white/45">{rest}</span>}
        </span>
        {withClose && (
          <button
            type="button"
            onClick={close}
            aria-label="Close"
            className="shrink-0 -mr-1 rounded p-1 text-white/50 transition-colors hover:bg-white/10 hover:text-white/90"
          >
            <X size={15} />
          </button>
        )}
      </div>
    );
  };

  if (overlay.mode === 'panel') {
    return (
      <>
        <div
          className="fixed z-[1000] bg-black/55"
          style={{ left: 0, right: 0, top: TITLE_BAR_HEIGHT, bottom: 0 }}
          onMouseDown={close}
        />
        <div
          className="fixed z-[1001] flex flex-col overflow-hidden rounded-xl border border-white/10 shadow-2xl"
          style={{ left: l.boxLeft, top: l.boxTop, width: l.boxW, height: l.boxH, background: TWITCH_PAGE_BG }}
        >
          {renderBar(true)}
          <div className="flex-1" style={{ background: TWITCH_PAGE_BG }} />
        </div>
      </>
    );
  }

  // fullbody (login / drops): takes over the app body below the title bar.
  return (
    <div
      className="fixed z-[1000] flex flex-col bg-[#0e0e10]"
      style={{ left: 0, right: 0, top: TITLE_BAR_HEIGHT, bottom: 0 }}
    >
      {renderBar(false)}
      <div className="flex-1 bg-[#0e0e10]" />
    </div>
  );
}
