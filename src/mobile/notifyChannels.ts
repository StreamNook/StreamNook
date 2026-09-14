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
import { useAppStore } from '../stores/AppStore';

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

  const { settings, updateSettings } = useAppStore.getState();
  const current = settings.live_notifications;
  // No settings loaded yet means the write would create a partial object that
  // overwrites real values with defaults on the next save.
  if (!current) return isChannelMuted(login);

  const list = current.muted_live_channels ?? [];
  const nowMuted = !list.includes(id);
  await updateSettings({
    ...settings,
    live_notifications: {
      ...current,
      muted_live_channels: nowMuted ? [...list, id] : list.filter((x) => x !== id),
    },
  });
  return nowMuted;
}
