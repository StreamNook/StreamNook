// Types for the plugin host UI. Mirrors the Rust types in
// src-tauri/src/plugin_host/ and the contract in docs/plugins/.

export type PluginTier = 'A' | 'B' | 'C';

export interface GrantedCaps {
  events: string[];
  host_methods: string[];
  credentials: string[];
  network: string;
  ui: string[];
  // Named hooks the plugin fills (see HOOKS.md).
  actions?: string[];
  status?: string[];
  provides?: string[];
}

export interface PluginInfo {
  id: string;
  name: string;
  version: string;
  author: string;
  tier: PluginTier;
  description: string;
  homepage: string | null;
  enabled: boolean;
  running: boolean;
  /** Runtime kind: "process" (separate executable) or "ui" (in-app module).
   *  For ui plugins, running mirrors enabled (the frontend loads them). */
  kind: 'process' | 'ui';
  /** True when the plugin contributes an in-app UI module the frontend loads:
   *  a ui plugin, or a process plugin with a ui_entry (hybrid). */
  has_ui: boolean;
  source: string;
  granted: GrantedCaps;
  credential_consent: Record<string, string>;
  has_panel: boolean;
}

export interface SourceInfo {
  url: string;
  name: string;
  operator: string;
  fingerprint: string;
  official: boolean;
}

/**
 * One source's catalogue.
 *
 * `entries` is already filtered to plugins this platform can install, so the
 * UI never offers something that would fail at the final click. `unavailableHere`
 * is how many the filter removed, and it exists because the two empty states
 * need different words: "your sources are unreachable" and "nothing is built
 * for your platform yet" send a user to debug entirely different things, and
 * both otherwise arrive as an empty array.
 */
export interface SourceListing {
  entries: IndexEntry[];
  unavailableHere: number;
  /** The `<os>-<arch>` the filter tested against, e.g. `linux-x86_64`. */
  platform: string;
}

export interface IndexEntry {
  id: string;
  name: string;
  version: string;
  tier: PluginTier;
  description: string;
  homepage: string | null;
  host_min: string;
  released_at: string | null;
  author: { name: string; pubkey: string; previous_pubkeys?: string[]; verified?: boolean };
  artifact: { url: string; sha256: string; size?: number; signature_url: string };
  // Marketplace metadata, all optional and presentation-only.
  icon_url?: string | null;
  banner_url?: string | null;
  readme_url?: string | null;
  downloads?: number | null;
  created_at?: string | null;
  updated_at?: string | null;
  /** First-party (built by StreamNook). Drives the "official" badge. Approved
   *  third-party plugins leave this false; being in the index is their approval. */
  official?: boolean;
}

/** Positive when a is newer than b. Semver-ish, tolerant of short versions. */
export function compareVersions(a: string, b: string): number {
  const pa = a.split('.').map((p) => parseInt(p, 10) || 0);
  const pb = b.split('.').map((p) => parseInt(p, 10) || 0);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const diff = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (diff !== 0) return diff;
  }
  return 0;
}

export interface PanelField {
  key: string;
  // Generic field types any plugin can declare. The host renders each one
  // with a rich native control; the plugin never ships UI.
  type:
    | 'toggle'
    | 'number'
    | 'select'
    | 'text'
    | 'string_list' // add-and-remove chip rows (not a textarea)
    | 'channel_list' // Twitch channel search picker (avatars, live dots)
    | 'slider' // range with a value readout
    | 'folder'; // directory path with a native browse dialog
  label: string;
  description?: string;
  default?: unknown;
  min?: number;
  max?: number;
  step?: number;
  /** Unit suffix shown next to a slider's value, e.g. "min". */
  unit?: string;
  /** Divides a slider's raw value for display (e.g. 60 to show seconds as minutes). */
  display_divisor?: number;
  placeholder?: string;
  options?: { value: string; label: string }[];
}

/** One channel entry stored by a `channel_list` field. */
export interface PanelChannel {
  channel_id: string;
  channel_login: string;
  display_name: string;
}

export interface PanelSection {
  label?: string;
  description?: string;
  fields: PanelField[];
}

export interface PanelSchema {
  title: string;
  sections: PanelSection[];
}

export type PanelValues = Record<string, boolean | number | string | string[]>;

// The exact consent lines from docs/plugins/CAPABILITIES.md. The dialog
// renders these verbatim from the granted (or requested) capability set.
export function capabilityLines(caps: GrantedCaps): { text: string; warning: boolean }[] {
  const lines: { text: string; warning: boolean }[] = [];
  const watchEvents = ['on_stream_start', 'on_stream_stop', 'on_channel_change', 'on_watch_tick'];
  if (caps.events.some((e) => watchEvents.includes(e))) {
    lines.push({ text: 'Knows which channel you are watching', warning: false });
  }
  if (caps.events.includes('on_followed_live')) {
    lines.push({ text: 'Sees which channels you follow are live', warning: false });
  }
  if (caps.events.includes('on_chat_message')) {
    lines.push({ text: 'Sees chat messages in channels you open', warning: false });
  }
  if (caps.events.includes('on_settings_change')) {
    lines.push({ text: 'Is told when certain app settings change', warning: false });
  }
  if (caps.host_methods.includes('get_followed_live')) {
    lines.push({ text: 'Can ask for your list of live followed channels', warning: false });
  }
  if (caps.host_methods.includes('set_upstream')) {
    lines.push({ text: 'Can supply the video source the player uses', warning: false });
  }
  if (caps.host_methods.includes('notify')) {
    lines.push({ text: 'Can show you notifications', warning: false });
  }
  if (caps.ui.includes('panel') || caps.host_methods.includes('register_panel')) {
    lines.push({ text: 'Adds a settings panel inside StreamNook', warning: false });
  }
  for (const kind of caps.credentials) {
    if (kind === 'twitch.android') {
      lines.push({
        text: 'Uses your Twitch login, so it can act on your account (watch, claim, and the like)',
        warning: true,
      });
    }
  }
  if (caps.network === 'external') {
    lines.push({
      text: 'Makes its own network connections; StreamNook does not route this traffic',
      warning: false,
    });
  } else if (caps.network === 'none') {
    lines.push({ text: 'Makes no network connections of its own', warning: false });
  }
  if (caps.provides && caps.provides.length > 0) {
    lines.push({ text: "Powers some of the app's built-in controls", warning: false });
  }
  return lines;
}

// Neutral, capability-scope labels. The tier is quiet curation metadata, not
// a risk rating.
export const TIER_LABEL: Record<PluginTier, string> = {
  A: 'Standard',
  B: 'Extended',
  C: 'Advanced',
};
