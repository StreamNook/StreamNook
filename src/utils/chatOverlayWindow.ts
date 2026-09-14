// Spawner for the transparent always-on-top chat overlay window (`#/chat-overlay`).
//
// Each overlay is its own WebView2 renderer (roughly 150 MB), so it is opened
// only by an explicit user action (header button, /overlay, palette). Labels
// are `overlay-w<n>` (first free), so several overlays can coexist and the
// Rust window handler counts them as popouts for tray and exit decisions.
//
// Windows note: an exclusive-fullscreen game hides every topmost window; the
// overlay only floats over borderless-windowed games.

import { Logger } from './logger';

export interface OpenChatOverlayOptions {
  channel: string;
  channelId?: string | null;
  channelName?: string | null;
}

export const OVERLAY_GEOMETRY_KEY = 'streamnook.chat-overlay-geometry';
const LABEL_PREFIX = 'overlay-w';
const MAX_OVERLAYS = 4;
const DEFAULT_WIDTH = 380;
const DEFAULT_HEIGHT = 520;

export async function openChatOverlayWindow(options: OpenChatOverlayOptions): Promise<void> {
  try {
    const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow');
    const { getCurrentWindow } = await import('@tauri-apps/api/window');

    const open = new Set<string>();
    try {
      for (const w of await WebviewWindow.getAll()) {
        if (w.label.startsWith(LABEL_PREFIX)) open.add(w.label);
      }
    } catch (err) {
      Logger.debug('[ChatOverlay] getAll failed:', err);
    }
    let n = 1;
    while (open.has(`${LABEL_PREFIX}${n}`) && n <= MAX_OVERLAYS) n += 1;
    if (n > MAX_OVERLAYS) {
      Logger.warn(`[ChatOverlay] ${MAX_OVERLAYS} overlays already open`);
      return;
    }
    const label = `${LABEL_PREFIX}${n}`;

    const params = new URLSearchParams({ channel: options.channel.toLowerCase() });
    if (options.channelId) params.set('channelId', options.channelId);
    if (options.channelName) params.set('channelName', options.channelName);

    let width = DEFAULT_WIDTH;
    let height = DEFAULT_HEIGHT;
    let x: number | undefined;
    let y: number | undefined;
    try {
      const { useAppStore } = await import('../stores/AppStore');
      const pref = useAppStore.getState().settings.chat_overlay;
      if (pref?.width) width = Math.max(240, Math.min(900, pref.width));
      if (pref?.height) height = Math.max(200, Math.min(1400, pref.height));
    } catch {
      /* defaults */
    }
    try {
      const geo = JSON.parse(localStorage.getItem(OVERLAY_GEOMETRY_KEY) || 'null');
      if (geo && typeof geo.x === 'number' && typeof geo.y === 'number') {
        const step = 32 * (n - 1);
        x = geo.x + step;
        y = geo.y + step;
        if (typeof geo.width === 'number') width = geo.width;
        if (typeof geo.height === 'number') height = geo.height;
      }
    } catch {
      /* no saved geometry */
    }
    if (x === undefined || y === undefined) {
      try {
        const main = getCurrentWindow();
        const pos = await main.outerPosition();
        const size = await main.outerSize();
        x = pos.x + size.width - width - 24;
        y = pos.y + 64;
      } catch {
        /* let the OS place it */
      }
    }

    const win = new WebviewWindow(label, {
      url: `${window.location.origin}/#/chat-overlay?${params.toString()}`,
      title: `StreamNook chat overlay: ${options.channelName ?? options.channel}`,
      width,
      height,
      x,
      y,
      resizable: true,
      decorations: false,
      transparent: true,
      alwaysOnTop: true,
      skipTaskbar: true,
      // Windows draws a 1px DWM frame line as part of the window shadow, even
      // on an undecorated window. That was the "second border" around the
      // glass slab; the slab paints its own single hairline instead.
      shadow: false,
      minimizable: false,
      maximizable: false,
      focus: true,
      dragDropEnabled: false,
    });
    win.once('tauri://error', (e) => {
      Logger.error('[ChatOverlay] Failed to open overlay window:', e);
    });
    Logger.debug(`[ChatOverlay] Opened ${label} for ${options.channel}`);
  } catch (err) {
    Logger.error('[ChatOverlay] openChatOverlayWindow failed:', err);
    throw err;
  }
}
