// Transparent, always-on-top chat overlay window (`#/chat-overlay`).
//
// A viewer, not a chat client: one channel's messages floating on a glass
// slab over a borderless game or any other app. No composer, no header
// chrome at rest. Hovering reveals a slim control strip (drag, opacity,
// click-through, close); leaving hides it again so only the messages float.
// "Click-through" hands every mouse event to the window underneath; since
// the overlay can then no longer be clicked, it is turned back from outside:
// the tray item "Make chat overlays clickable", Ctrl+Alt+N in the main
// window, or the palette entry. The window listens for
// `chat-overlay-toggle-interactive`.
//
// Same Rust core as every other surface: the channel is acquired on the
// shared IRC connection and rows carry the rule-engine stamps.

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { listen } from '@tauri-apps/api/event';
import { MousePointer2, MousePointerBan, X } from 'lucide-react';
import BlendedChatPane from './BlendedChatPane';
import { Tooltip } from '../ui/Tooltip';
import { useAppStore } from '../../stores/AppStore';
import { acquireChannel, releaseChannel } from '../../stores/chatConnectionStore';
import {
  applyTheme,
  applyGlassStrength,
  applyFont,
  getThemeById,
  getThemeByIdWithCustom,
  getOledTheme,
  DEFAULT_THEME_ID,
  DEFAULT_GLASS_TRANSPARENCY,
  DEFAULT_FONT_ID,
  OLED_THEME_ID,
} from '../../themes';
import { listenForSettingsUpdates } from '../../utils/settingsBroadcast';
import { OVERLAY_GEOMETRY_KEY } from '../../utils/chatOverlayWindow';
import { Logger } from '../../utils/logger';

function readParams() {
  const hash = window.location.hash;
  const q = hash.indexOf('?');
  const params = new URLSearchParams(q >= 0 ? hash.slice(q + 1) : '');
  return {
    channel: (params.get('channel') ?? '').toLowerCase(),
    channelId: params.get('channelId') ?? '',
    channelName: params.get('channelName') ?? params.get('channel') ?? '',
  };
}

const CHROME_LINGER_MS = 1600;

export default function ChatOverlayWindow() {
  const { channel, channelId, channelName } = useMemo(() => readParams(), []);
  const settings = useAppStore((s) => s.settings);
  const updateSettings = useAppStore((s) => s.updateSettings);
  const [opacity, setOpacity] = useState<number>(() => settings.chat_overlay?.opacity ?? 70);
  const [clickThrough, setClickThrough] = useState(false);
  // Control strip visibility: shown while the pointer is over the window and
  // for a moment after it leaves, so the strip never pops in and out.
  const [chrome, setChrome] = useState(false);
  const chromeTimer = useRef<number | null>(null);
  const opacityTimer = useRef<number | null>(null);

  // Transparent ground: document and React root must not paint.
  useEffect(() => {
    const html = document.documentElement;
    const body = document.body;
    const root = document.getElementById('root');
    const prev = [html.style.background, body.style.background, root?.style.background ?? ''];
    html.style.background = 'transparent';
    body.style.background = 'transparent';
    if (root) root.style.background = 'transparent';
    html.classList.add('sn-chat-overlay');
    return () => {
      html.style.background = prev[0];
      body.style.background = prev[1];
      if (root) root.style.background = prev[2];
      html.classList.remove('sn-chat-overlay');
    };
  }, []);

  // Boot like the MultiChat popout: settings, auth, cross-window sync.
  useEffect(() => {
    const store = useAppStore.getState();
    void store.loadSettings().catch((err) => Logger.warn('[ChatOverlay] loadSettings failed:', err));
    void store.checkAuthStatus().catch((err) => Logger.warn('[ChatOverlay] checkAuthStatus failed:', err));
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void listenForSettingsUpdates(() => {
      void useAppStore.getState().loadSettings();
    }).then((u) => {
      if (cancelled) u();
      else unlisten = u;
    });
    return () => {
      cancelled = true;
      void Promise.resolve(unlisten?.()).catch(() => {});
    };
  }, []);

  useEffect(() => {
    const themeId = settings.theme || DEFAULT_THEME_ID;
    const theme =
      themeId === OLED_THEME_ID
        ? getOledTheme(settings.oled_accent)
        : getThemeByIdWithCustom(themeId, settings.custom_themes || []) || getThemeById(DEFAULT_THEME_ID);
    if (theme) applyTheme(theme);
    applyGlassStrength(settings.glass_transparency ?? DEFAULT_GLASS_TRANSPARENCY);
    applyFont(settings.font ?? DEFAULT_FONT_ID, settings.font_custom);
  }, [settings.theme, settings.custom_themes, settings.glass_transparency, settings.font, settings.font_custom, settings.oled_accent]);

  // Join through the shared Rust connection; release on close.
  useEffect(() => {
    if (!channel) return;
    void acquireChannel(channel, channelId || null, 'twitch').catch((err) =>
      Logger.warn('[ChatOverlay] acquireChannel failed:', err),
    );
    return () => {
      void releaseChannel(channel, 'twitch').catch(() => {});
    };
  }, [channel, channelId]);

  // Persist geometry (debounced) so the next overlay lands in the same spot.
  useEffect(() => {
    let timer: number | null = null;
    let unlistenMove: (() => void) | undefined;
    let unlistenResize: (() => void) | undefined;
    const win = getCurrentWindow();
    const save = () => {
      if (timer) window.clearTimeout(timer);
      timer = window.setTimeout(async () => {
        try {
          const pos = await win.outerPosition();
          const size = await win.outerSize();
          localStorage.setItem(
            OVERLAY_GEOMETRY_KEY,
            JSON.stringify({ x: pos.x, y: pos.y, width: size.width, height: size.height }),
          );
          const cur = useAppStore.getState().settings;
          updateSettings({
            ...cur,
            chat_overlay: { ...cur.chat_overlay, width: size.width, height: size.height },
          });
        } catch {
          /* ignore */
        }
      }, 400);
    };
    void win.onMoved(save).then((u) => (unlistenMove = u));
    void win.onResized(save).then((u) => (unlistenResize = u));
    return () => {
      if (timer) window.clearTimeout(timer);
      unlistenMove?.();
      unlistenResize?.();
    };
  }, [updateSettings]);

  const applyClickThrough = useCallback(async (on: boolean) => {
    try {
      await getCurrentWindow().setIgnoreCursorEvents(on);
      setClickThrough(on);
    } catch (err) {
      Logger.warn('[ChatOverlay] setIgnoreCursorEvents failed:', err);
    }
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void listen<{ interactive?: boolean } | null>('chat-overlay-toggle-interactive', (e) => {
      const want = e.payload && typeof e.payload.interactive === 'boolean' ? !e.payload.interactive : !clickThrough;
      void applyClickThrough(want);
    }).then((u) => {
      if (cancelled) u();
      else unlisten = u;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [applyClickThrough, clickThrough]);

  const showChrome = () => {
    if (chromeTimer.current) window.clearTimeout(chromeTimer.current);
    setChrome(true);
  };
  const hideChromeSoon = () => {
    if (chromeTimer.current) window.clearTimeout(chromeTimer.current);
    chromeTimer.current = window.setTimeout(() => setChrome(false), CHROME_LINGER_MS);
  };
  useEffect(() => () => {
    if (chromeTimer.current) window.clearTimeout(chromeTimer.current);
  }, []);

  const onOpacity = (v: number) => {
    setOpacity(v);
    if (opacityTimer.current) window.clearTimeout(opacityTimer.current);
    opacityTimer.current = window.setTimeout(() => {
      const cur = useAppStore.getState().settings;
      updateSettings({ ...cur, chat_overlay: { ...cur.chat_overlay, opacity: v } });
    }, 300);
  };

  const channels = useMemo(
    () => [{ channel, channelId: channelId || null, channelName: channelName || channel, provider: 'twitch' as const }],
    [channel, channelId, channelName],
  );

  if (!channel) {
    return <div className="p-3 text-xs text-textSecondary">No channel given.</div>;
  }

  return (
    <div
      className="sn-chat-overlay-frame flex h-screen w-screen flex-col overflow-hidden"
      data-chrome={chrome ? 'on' : 'off'}
      data-clickthrough={clickThrough ? 'on' : 'off'}
      onMouseEnter={showChrome}
      onMouseMove={showChrome}
      onMouseLeave={hideChromeSoon}
      style={{ ['--sn-overlay-alpha' as string]: String(Math.max(0, Math.min(100, opacity)) / 100) }}
    >
      {/* Control strip: drag handle + opacity + click-through + close. Hidden
          at rest; the whole strip is the drag region except its controls. */}
      <div data-tauri-drag-region className="sn-chat-overlay-chrome flex h-[30px] flex-shrink-0 select-none items-center gap-2 px-2.5">
        <span data-tauri-drag-region className="pointer-events-none flex min-w-0 items-center gap-1.5">
          <span className="h-1.5 w-1.5 flex-shrink-0 rounded-full bg-error/90 shadow-[0_0_6px_var(--color-error)]" />
          <span className="truncate text-[11px] font-semibold tracking-wide text-textPrimary/85">{channelName}</span>
        </span>
        <span data-tauri-drag-region className="flex-1" />
        <div className="flex items-center gap-1" data-tauri-drag-region="false">
          <Tooltip content={`Glass ${opacity}%`} side="bottom">
            <input
              type="range"
              min={0}
              max={100}
              step={5}
              value={opacity}
              onChange={(e) => onOpacity(Number(e.target.value))}
              className="sn-overlay-slider h-1 w-16"
              aria-label="Overlay opacity"
            />
          </Tooltip>
          <Tooltip
            content={clickThrough ? 'Click-through is on. Tray: Make chat overlays clickable' : 'Click-through: send clicks to the window underneath'}
            side="bottom"
          >
            <button
              type="button"
              onClick={() => void applyClickThrough(!clickThrough)}
              className={`glass-button grid h-[22px] w-[22px] place-items-center ${clickThrough ? 'text-accent' : 'text-textSecondary hover:text-textPrimary'}`}
              aria-label="Toggle click-through"
            >
              {clickThrough ? <MousePointerBan size={12} /> : <MousePointer2 size={12} />}
            </button>
          </Tooltip>
          <Tooltip content="Close overlay" side="bottom">
            <button
              type="button"
              onClick={() => void getCurrentWindow().close()}
              className="glass-button grid h-[22px] w-[22px] place-items-center text-textSecondary hover:text-error"
              aria-label="Close overlay"
            >
              <X size={12} />
            </button>
          </Tooltip>
        </div>
      </div>
      <div className="sn-overlay-feed flex min-h-0 flex-1 flex-col">
        <BlendedChatPane channels={channels} readOnly transparent />
      </div>
    </div>
  );
}
