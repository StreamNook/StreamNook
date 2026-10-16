// Field-level settings writes.
//
// `set_settings_fields` changes each dotted path (`video_player.ad_bypass_enabled`)
// as a read-modify-write on the settings Rust holds, so a page holding an older
// copy of a group never writes the rest of that group back. Rust announces the
// write with no sender, so every window, this one included, re-reads its store
// through the settings sync listener.

import { invoke } from '@tauri-apps/api/core';

/** Write these dotted paths, and only these. A null value removes the key. */
export function setSettingsFields(changes: Record<string, unknown>): Promise<void> {
  if (Object.keys(changes).length === 0) return Promise.resolve();
  return invoke('set_settings_fields', { changes });
}

/** `{a: 1}` under `group` as `{'group.a': 1}`. */
export function fieldsIn(group: string, changes: Record<string, unknown>): Record<string, unknown> {
  return Object.fromEntries(Object.entries(changes).map(([k, v]) => [`${group}.${k}`, v ?? null]));
}
