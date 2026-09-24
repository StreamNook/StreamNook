// Cross-window settings sync.
//
// Rust holds the canonical settings. A window writes only the top-level keys it
// changed (`patchSettings`), and Rust announces every write with one event that
// every window listens for and refreshes its store on. A window never sends its
// whole settings object: one holding an older copy would revert whatever another
// window had saved in the meantime.

import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { Settings } from '../types';
import { Logger } from './logger';

export const SETTINGS_UPDATED_EVENT = 'streamnook-settings-updated';

// A per-window-load random id. Stamped onto every emitted update so the
// originating window can ignore its own broadcast (no need to re-read the
// settings it just wrote).
export const SENDER_ID =
  typeof crypto !== 'undefined' && 'randomUUID' in crypto
    ? crypto.randomUUID()
    : `${Date.now()}-${Math.random().toString(36).slice(2)}`;

export interface SettingsUpdatedPayload {
  source: string | null;
  keys: string[];
}

/** The top-level settings keys to write, each with its new value. `null`
 *  clears a key back to its default (an `undefined` would not survive JSON). */
export type SettingsPatch = { [K in keyof Settings]?: Settings[K] | null };

/** Write these keys, and only these, to the canonical settings. */
export function patchSettings(patch: SettingsPatch): Promise<void> {
  if (Object.keys(patch).length === 0) return Promise.resolve();
  return invoke('patch_settings', { patch, source: SENDER_ID });
}

/** The keys whose values differ between two settings objects, as a patch. */
export function settingsDiff(before: Settings, after: Settings): SettingsPatch {
  const patch: Record<string, unknown> = {};
  const keys = new Set([...Object.keys(before), ...Object.keys(after)]);
  for (const key of keys) {
    const was = (before as unknown as Record<string, unknown>)[key];
    const now = (after as unknown as Record<string, unknown>)[key];
    if (was === now) continue;
    if (JSON.stringify(was) !== JSON.stringify(now)) patch[key] = now ?? null;
  }
  return patch as SettingsPatch;
}

// Subscribe a callback to settings-updated events from OTHER windows. Returns
// an unlisten function — call it on component unmount to detach.
export async function listenForSettingsUpdates(
  onUpdate: () => void,
): Promise<UnlistenFn> {
  return listen<SettingsUpdatedPayload>(SETTINGS_UPDATED_EVENT, (event) => {
    if (event.payload?.source === SENDER_ID) return;
    try {
      onUpdate();
    } catch (err) {
      Logger.warn('[SettingsBroadcast] onUpdate handler threw:', err);
    }
  });
}
