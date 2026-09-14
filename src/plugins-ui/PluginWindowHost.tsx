// Standalone OS-window shell for a ui plugin's popout surface, routed via the
// `#/plugin/<pluginId>/<surface>` hash in main.tsx. The host owns the chrome:
// a frameless titlebar (logo, title, keep-on-top pin, minimize, close), the
// user's theme, and tooltips. The plugin's module supplies only the content
// component via its windowSurface export.
//
// Each Tauri window has its own JS context, so this shell hydrates settings
// (for the theme) on mount and keeps them fresh when other windows save.

import { useEffect, useMemo, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Pin, PinOff, X } from 'lucide-react';
import { Minus } from 'phosphor-react';
import type * as React from 'react';
import { useAppStore } from '../stores/AppStore';
import { listenForSettingsUpdates } from '../utils/settingsBroadcast';
import { TooltipManager } from '../components/ui/TooltipManager';
import { Tooltip } from '../components/ui/Tooltip';
import { Logger } from '../utils/logger';
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
} from '../themes';
import streamNookLogoUrl from '../assets/streamnook-logo-128.webp';
import { loadWindowSurface } from './loader';

interface WindowRoute {
  pluginId: string;
  surface: string;
  title: string;
}

/** Parses `#/plugin/<pluginId>/<surface>?title=...`. */
function parseRoute(hash: string): WindowRoute | null {
  const match = hash.match(/^#\/plugin\/([^/]+)\/([^/?]+)(?:\?(.*))?$/);
  if (!match) return null;
  const params = new URLSearchParams(match[3] ?? '');
  return {
    pluginId: decodeURIComponent(match[1]),
    surface: decodeURIComponent(match[2]),
    title: params.get('title') ?? 'Plugin',
  };
}

export const PluginWindowHost = () => {
  const route = useMemo(() => parseRoute(window.location.hash), []);
  const settings = useAppStore((s) => s.settings);
  const [pinned, setPinned] = useState(false);
  const [Surface, setSurface] = useState<React.ComponentType | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  // Hydrate settings (this window's store boots empty) and keep them fresh
  // when any other window saves.
  useEffect(() => {
    const store = useAppStore.getState();
    void store.loadSettings().catch((err) => {
      Logger.warn('[PluginWindow] loadSettings failed:', err);
    });
    let unlistenSettings: (() => void) | undefined;
    let cancelled = false;
    void listenForSettingsUpdates(() => {
      void useAppStore.getState().loadSettings();
    }).then((u) => {
      if (cancelled) {
        u();
        return;
      }
      unlistenSettings = u;
    });
    return () => {
      cancelled = true;
      unlistenSettings?.();
    };
  }, []);

  // Apply the user's theme so the popout matches the main app.
  useEffect(() => {
    const themeId = settings.theme || DEFAULT_THEME_ID;
    const theme =
      themeId === OLED_THEME_ID
        ? getOledTheme(settings.oled_accent)
        : getThemeByIdWithCustom(themeId, settings.custom_themes || []) ||
          getThemeById(DEFAULT_THEME_ID);
    if (theme) applyTheme(theme);
    applyGlassStrength(settings.glass_transparency ?? DEFAULT_GLASS_TRANSPARENCY);
    applyFont(settings.font ?? DEFAULT_FONT_ID, settings.font_custom);
  }, [
    settings.theme,
    settings.custom_themes,
    settings.glass_transparency,
    settings.font,
    settings.font_custom,
    settings.oled_accent,
  ]);

  // Load the plugin module and resolve this surface's component.
  useEffect(() => {
    if (!route) return;
    let cancelled = false;
    let dispose: (() => void) | undefined;
    void loadWindowSurface(route.pluginId, route.surface)
      .then((result) => {
        if (cancelled) {
          result.dispose();
          return;
        }
        dispose = result.dispose;
        if (result.Component) {
          setSurface(() => result.Component);
        } else {
          setLoadError('This plugin does not provide a window surface.');
        }
      })
      .catch((err) => {
        Logger.error('[PluginWindow] surface load failed:', err);
        if (!cancelled) setLoadError('The plugin failed to load.');
      });
    return () => {
      cancelled = true;
      dispose?.();
    };
  }, [route]);

  const togglePin = async () => {
    try {
      await getCurrentWindow().setAlwaysOnTop(!pinned);
      setPinned(!pinned);
    } catch (err) {
      Logger.warn('[PluginWindow] setAlwaysOnTop failed:', err);
    }
  };

  const minimize = () => void getCurrentWindow().minimize();
  const close = () => void getCurrentWindow().close();

  return (
    <div className="flex h-screen w-screen flex-col overflow-hidden bg-background text-textPrimary">
      {/* Titlebar */}
      <div
        data-tauri-drag-region
        className="relative z-50 flex h-[33px] select-none items-center justify-between border-b border-borderSubtle bg-secondary px-3 shrink-0"
      >
        <div data-tauri-drag-region className="pointer-events-none flex items-center gap-2">
          <img
            src={streamNookLogoUrl}
            alt="StreamNook"
            className="h-4 w-4 object-contain"
            draggable={false}
          />
          <span className="text-xs font-semibold tracking-wide text-textSecondary">
            {route?.title ?? 'Plugin'}
          </span>
        </div>
        <div className="flex space-x-1" style={{ WebkitAppRegion: 'no-drag' } as React.CSSProperties}>
          <Tooltip content={pinned ? 'Unpin from top' : 'Keep on top'} delay={200}>
            <button
              type="button"
              onClick={() => void togglePin()}
              data-tauri-drag-region="false"
              aria-pressed={pinned}
              className={`p-1.5 rounded transition-all duration-200 ${
                pinned ? 'text-accent' : 'text-textSecondary hover:text-textPrimary'
              }`}
            >
              {pinned ? <PinOff size={14} /> : <Pin size={14} />}
            </button>
          </Tooltip>
          <Tooltip content="Minimize" delay={200}>
            <button
              type="button"
              onClick={minimize}
              data-tauri-drag-region="false"
              className="p-1.5 text-textSecondary hover:text-textPrimary rounded transition-all duration-200"
            >
              <Minus size={14} />
            </button>
          </Tooltip>
          <Tooltip content="Close" delay={200}>
            <button
              type="button"
              onClick={close}
              data-tauri-drag-region="false"
              className="p-1.5 text-textSecondary hover:text-red-400 rounded transition-all duration-200"
            >
              <X size={14} />
            </button>
          </Tooltip>
        </div>
      </div>

      {/* Plugin surface fills the window; the OS handles resize from any edge. */}
      <div className="flex-1 min-h-0">
        {Surface ? (
          <Surface />
        ) : (
          <div className="flex h-full items-center justify-center px-6 text-center">
            <p className="text-xs text-textMuted">{loadError ?? 'Loading...'}</p>
          </div>
        )}
      </div>

      <TooltipManager />
    </div>
  );
};

export default PluginWindowHost;
