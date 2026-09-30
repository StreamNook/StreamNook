// Popout OS windows for ui plugins. The host owns the window: Rust builds a
// frameless window routed to the generic `#/plugin/<id>/<surface>` hash
// (open_plugin_window in commands/popout_window.rs), where PluginWindowHost
// renders the standard titlebar and theme and mounts the component the
// plugin's module returns from windowSurface(). Rust also places it, in
// physical pixels and wholly on screen, and remembers where it was.
//
// One window per (plugin, surface): reopening focuses the existing one.

import { invoke } from '@tauri-apps/api/core';
import { Logger } from '../utils/logger';
import type { PluginWindowOptions } from './types';

export async function openPluginWindow(
  pluginId: string,
  options: PluginWindowOptions,
): Promise<void> {
  try {
    await invoke('open_plugin_window', {
      request: {
        pluginId,
        surface: options.surface,
        title: options.title,
        width: options.width ?? null,
        height: options.height ?? null,
        minWidth: options.minWidth ?? null,
        minHeight: options.minHeight ?? null,
      },
    });
  } catch (err) {
    Logger.error('[PluginWindow] openPluginWindow failed:', err);
    throw err;
  }
}
