// Types for the in-app (runtime kind "ui") plugin engine: the contributions a
// plugin can register and the api object handed to its module. The contract
// is documented in docs/plugins/UI_PLUGINS.md; additive changes only.

import type * as React from 'react';
import type * as ReactJsxRuntime from 'react/jsx-runtime';
import type * as FramerMotion from 'framer-motion';
import type { TooltipProps } from '../components/ui/Tooltip';
import type { UiKit } from '../components/plugins/uiKit';

/** A button in the title bar's action cluster, rendered in native style. */
export interface TitleBarButtonContribution {
  id: string;
  tooltip: string;
  Icon: React.ComponentType<{ size?: number | string; className?: string }>;
  onClick: () => void;
  /** Optional React hook: whether the button shows the accent tint. */
  useIsActive?: () => boolean;
  /** Optional React hook: whether the button is shown at all. Lets a plugin
   *  gate its title-bar presence on one of its own settings. Defaults to
   *  always visible. */
  useIsVisible?: () => boolean;
}

/** A component mounted at the app root, above the main layout. The component
 *  owns its own visibility (render null while closed). */
export interface OverlayContribution {
  id: string;
  Component: React.ComponentType;
}

/** A contribution to a named slot a host feature exposes (e.g. a column
 *  docked into the Moderator Logs pane). The owning feature decides how the
 *  contribution renders and persists its own per-contribution preferences. */
export interface SlotContribution {
  id: string;
  /** Short lowercase noun used in host-generated copy ("Show lists in this pane"). */
  label: string;
  Icon: React.ComponentType<{ size?: number | string; className?: string }>;
  Component: React.ComponentType;
}

/** A channel returned by the host's drops channel picker. */
export interface PickedDropChannel {
  login: string;
  displayName: string;
  userId: string;
}

export const DROPS_CARD_ACTION_SLOT = 'drops.card-action';
/**
 * Context the host passes to a per-campaign control rendered in the
 * `drops.card-action` slot. A provider contributes a component here to hang its
 * own control (e.g. a start/stop button) on each Drops-center campaign card; the
 * host stays generic and renders whatever the provider hangs there, passing the
 * campaign it belongs to. The component does its work through its own actions —
 * the host neither knows nor names them.
 */
export interface DropCardActionContext {
  campaignId: string;
  campaignName: string;
  gameName: string;
  /** True when this campaign has time-earned drops (vs event/badge-only). */
  earnable: boolean;
  /** True when this campaign is the one currently progressing. */
  progressing: boolean;
  /** True when this campaign only drops on specific allow-listed channels. */
  isAclBased: boolean;
  /** The allow-listed channels (id + login) when isAclBased; empty otherwise. */
  allowedChannels: { id: string; name: string }[];
  /** Open the host's ACL-aware channel picker (same one core uses) and resolve
   *  with the chosen live channel, or null if dismissed. Lets a provider offer
   *  the "pick a specific channel" flow without reimplementing ACL/live logic. */
  pickChannel: () => Promise<PickedDropChannel | null>;
}

/** A command palette row supplied by a plugin provider. */
export interface PluginPaletteItem {
  id: string;
  title: string;
  subtitle?: string;
  keywords?: string;
  /** Single letter shown as the row's fallback avatar tile. */
  initial?: string;
  run: () => void;
}

/** A bindable command: dispatches globally, appears in Keybindings settings,
 *  and user rebinds persist under its id. */
export interface PluginKeybinding {
  id: string;
  label: string;
  description?: string;
  category?: string;
  defaultBindings: string[];
  keywords?: string;
  run: () => void;
}

export interface PluginWindowOptions {
  /** Names the window's content; passed to the module's windowSurface. */
  surface: string;
  title: string;
  width?: number;
  height?: number;
  minWidth?: number;
  minHeight?: number;
}

/** Everything a ui plugin can reach. Frozen, built per plugin. */
export interface PluginApi {
  pluginId: string;

  /** The host's copies of shared libraries. Plugin builds alias their bare
   *  imports to these so contributed components run on the host React tree. */
  libs: {
    react: typeof React;
    reactJsxRuntime: typeof ReactJsxRuntime;
    framerMotion: typeof FramerMotion;
  };

  /** Native components plugins should reuse instead of rebuilding: the Tooltip
   *  plus the settings UI kit (toggles, chip lists, sliders, the channel
   *  search-picker, section/row layout) so a plugin's own panel looks native. */
  components: {
    Tooltip: React.FC<TooltipProps>;
  } & UiKit;

  ui: {
    registerTitleBarButton: (contribution: TitleBarButtonContribution) => void;
    registerOverlay: (contribution: OverlayContribution) => void;
    registerSlot: (slotId: string, contribution: SlotContribution) => void;
  };

  commands: {
    registerKeybinding: (command: PluginKeybinding) => void;
    registerPaletteItems: (provider: () => PluginPaletteItem[]) => void;
  };

  settings: {
    /** Register this plugin's own settings component, rendered on its card in
     *  the plugins page. The plugin ships the UI; the host just mounts it. */
    registerPanel: (Component: React.ComponentType) => void;
  };

  windows: {
    open: (options: PluginWindowOptions) => Promise<void>;
  };

  events: {
    emit: (name: string, payload?: unknown) => Promise<void>;
    listen: (name: string, handler: (payload: unknown) => void) => Promise<() => void>;
  };

  chat: {
    /** React hook: true when a chat compose box exists in this window. */
    useHasTarget: () => boolean;
    /** Insert into the chat compose box at the caret; false when none mounted. */
    insertText: (text: string) => boolean;
  };

  log: {
    debug: (...args: unknown[]) => void;
    warn: (...args: unknown[]) => void;
    error: (...args: unknown[]) => void;
  };
}

/** The shape of a ui plugin's bundled module (UI_PLUGINS.md). */
export interface PluginUiModule {
  activate?: (api: PluginApi) => void | (() => void);
  deactivate?: () => void;
  windowSurface?: (surfaceId: string, api: PluginApi) => React.ComponentType | null;
}
