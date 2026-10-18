// Per-channel live-alert opt-out.
//
// Stored as an EXCLUSION set rather than an inclusion one, which is the whole
// product decision in one line: everyone keeps getting alerts for everything
// they follow, and an upgrade changes nothing until someone deliberately
// silences a channel. An inclusion list would have silently switched every
// existing user to receiving nothing.
//
// Kept out of notifications.ts on purpose: that module is about the OS surface
// (permission, channels, posting) and has no business reading the settings
// store.
//
// Rust owns the list: a toggle is one edit it applies to the stored set
// (logins trimmed and lowercased there), and every window's store re-reads it.
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../stores/AppStore';
import { Logger } from '../utils/logger';

/** Twitch logins are already lowercase, but nothing guarantees a caller's is. */
function key(login: string): string {
  return login.trim().toLowerCase();
}

function mutedList(): string[] {
  return useAppStore.getState().settings.live_notifications?.muted_live_channels ?? [];
}

export function isChannelMuted(login: string): boolean {
  if (!login) return false;
  return mutedList().includes(key(login));
}

/** Returns the new muted state, so callers can render optimistically. */
export async function toggleChannelMuted(login: string): Promise<boolean> {
  const id = key(login);
  if (!id) return false;

  // Before settings load there is no stored list to edit yet, so the current
  // state is the answer.
  if (!useAppStore.getState().settings.live_notifications) return isChannelMuted(login);

  try {
    const stored = await invoke<string[]>('edit_settings_list', {
      path: 'live_notifications.muted_live_channels',
      edit: { op: 'toggle', value: id },
    });
    return stored.includes(id);
  } catch (err) {
    Logger.warn('[NotifyChannels] mute toggle failed:', err);
    return isChannelMuted(login);
  }
}
