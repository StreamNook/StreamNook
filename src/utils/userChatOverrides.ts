// Per-user chat overrides (nickname + color). Lookups are sync against the
// `chat_customization.user_overrides` Record on the global settings store, so
// callers can read inside render without effects. Writes go through Rust's
// `set_chat_user_override` (see `writeOverride` below).

import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../stores/AppStore';
import { Logger } from './logger';
import type { UserChatOverride } from '../types';

type OverrideMap = Record<string, UserChatOverride>;

export function getUserOverride(
  userId: string | null | undefined,
  overrides: OverrideMap | undefined,
): UserChatOverride | undefined {
  if (!userId || !overrides) return undefined;
  return overrides[userId];
}

// Returns the nickname when one is set, else falls back to the supplied
// display name (typically the Twitch display-name tag or username).
export function getDisplayedName(
  userId: string | null | undefined,
  fallback: string,
  overrides: OverrideMap | undefined,
): string {
  const override = getUserOverride(userId, overrides);
  if (override?.nickname && override.nickname.trim().length > 0) {
    return override.nickname;
  }
  return fallback;
}

// Returns the color override when one is set, else null (caller falls back).
export function getColorOverride(
  userId: string | null | undefined,
  overrides: OverrideMap | undefined,
): string | null {
  const override = getUserOverride(userId, overrides);
  return override?.color && override.color.trim().length > 0 ? override.color : null;
}

// Snapshot of the current override map. Use sparingly — readers that
// re-render on changes should pull from `settings` via the AppStore hook,
// not from this getter (which is a one-shot read).
export function snapshotOverrides(): OverrideMap {
  return useAppStore.getState().settings.chat_customization?.user_overrides ?? {};
}

// Every write is a read-modify-write in Rust on the canonical settings, never a
// patch of this window's copy of the group: that copy can be stale (a popout
// still loading settings, or an earlier edit still in flight), and the group
// travels whole, so a stale copy erased every other saved override. Rust
// announces the write and every window, this one included, reloads settings.
function writeOverride(
  userId: string,
  username: string | null,
  field: 'nickname' | 'color' | null,
  value: string | null,
): void {
  if (!userId) return;
  invoke('set_chat_user_override', { userId, username, field, value }).catch((err) =>
    Logger.warn('[UserOverrides] set_chat_user_override failed:', err),
  );
}

// A blank nickname clears it. `username` is kept so the Settings UI can render
// "Bob → Robert" without an API roundtrip.
export function setUserNickname(userId: string, username: string, nickname: string | null): void {
  writeOverride(userId, username, 'nickname', nickname);
}

export function setUserColor(userId: string, username: string, color: string | null): void {
  writeOverride(userId, username, 'color', color);
}

// Drops the entire override entry for a user.
export function clearUserOverride(userId: string): void {
  writeOverride(userId, null, null, null);
}
