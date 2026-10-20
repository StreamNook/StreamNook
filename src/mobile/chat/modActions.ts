// Twitch moderator helpers for the phone's long-press sheet. Delete, timeout
// and ban go through `moderateWithToast` (services/moderation), the same door
// desktop's hover dock uses; pinning and the per-channel mod check live here.
import { invoke } from '@tauri-apps/api/core';
import { Logger } from '../../utils/logger';

/** Mod powers are read off your own badges in that channel, same rule as desktop. */
export function isModeratorFrom(userBadges: string | null | undefined): boolean {
  if (!userBadges) return false;
  return /(?:^|,)(?:moderator|broadcaster)\//.test(userBadges);
}

export async function pinMessage(broadcasterId: string, messageId: string): Promise<boolean> {
  try {
    await invoke('pin_chat_message', { broadcasterId, messageId, durationSeconds: null });
    return true;
  } catch (err) {
    Logger.error('[MobileMod] pin failed:', err);
    return false;
  }
}

export const TIMEOUT_OPTIONS: { label: string; seconds: number }[] = [
  { label: '60s', seconds: 60 },
  { label: '10m', seconds: 600 },
  { label: '1h', seconds: 3600 },
  { label: '24h', seconds: 86400 },
];
