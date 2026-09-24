import { invoke } from '@tauri-apps/api/core';

/**
 * Thin wrappers over the Kick / YouTube account commands, mirroring
 * `accountService.ts`'s role for Twitch.
 *
 * Before this existed, every one of these was a raw `invoke` written out at the
 * call site — in ChatWidget, BlendedChatPane, ConnectionsSettings and
 * followsStore — which is why the same account could be connected two different
 * ways with two different outcomes. One place, one spelling.
 */

/** Platforms with an account you can connect. Twitch is not one of these: it is
 *  the app's native account and lives in `accountService`. */
export type PlatformId = 'kick' | 'youtube' | 'tiktok';

const IS_CONNECTED: Record<PlatformId, string> = {
  kick: 'kick_is_connected',
  youtube: 'youtube_is_connected',
  tiktok: 'tiktok_is_connected',
};

const DISCONNECT: Record<PlatformId, string> = {
  kick: 'kick_disconnect',
  youtube: 'youtube_disconnect',
  tiktok: 'tiktok_disconnect',
};

export function isConnected(provider: PlatformId): Promise<boolean> {
  return invoke<boolean>(IS_CONNECTED[provider]);
}

export interface PlatformAccountInfo {
  name: string | null;
  avatar_url: string | null;
  /**
   * The id this platform's chat identifies the account by: Kick's numeric
   * account id, YouTube's `UC…` channel id. It is what lets a member's
   * StreamNook cosmetics follow them into that platform's chat.
   *
   * `null` is normal, not an error. A YouTube account can have no channel, and
   * a session stored before this existed fills it on the next read.
   */
  id: string | null;
  /** The @handle, sent only where a platform has one apart from the display
   *  name (TikTok). */
  handle?: string | null;
}

/**
 * Who is signed in on a platform — display name, profile picture and account id.
 *
 * One call, because all three come out of the same upstream response. Asking for
 * them separately would be extra authenticated round trips for something we
 * already had in hand.
 */
export function accountInfo(provider: PlatformId): Promise<PlatformAccountInfo> {
  return invoke<PlatformAccountInfo>('platform_account_info', { provider });
}

export function disconnect(provider: PlatformId): Promise<void> {
  return invoke<void>(DISCONNECT[provider]);
}

/**
 * Open the TikTok sign-in overlay and keep the session. That is the whole of
 * connecting TikTok: the session exists to play age-restricted LIVEs, and
 * nothing is imported.
 */
export function beginTiktokSession(): Promise<void> {
  return invoke<void>('tiktok_connect');
}

/**
 * Open the YouTube sign-in overlay and harvest the session.
 *
 * Only half of connecting YouTube — the channels still have to be read in
 * afterwards. Callers should use `platformAccountStore.connect('youtube')`, which
 * does both; this is exported for it, not for direct use.
 */
export function beginYoutubeSession(): Promise<void> {
  return invoke<void>('youtube_connect');
}
