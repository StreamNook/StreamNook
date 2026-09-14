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
export type PlatformId = 'kick' | 'youtube';

export function isConnected(provider: PlatformId): Promise<boolean> {
  return invoke<boolean>(provider === 'kick' ? 'kick_is_connected' : 'youtube_is_connected');
}

export interface PlatformAccountInfo {
  name: string | null;
  avatar_url: string | null;
}

/**
 * Who is signed in on a platform — display name AND profile picture.
 *
 * One call, because both come out of the same upstream response. Asking for the
 * name and then the avatar separately would be a second authenticated round trip
 * for something we already had in hand.
 */
export function accountInfo(provider: PlatformId): Promise<PlatformAccountInfo> {
  return invoke<PlatformAccountInfo>('platform_account_info', { provider });
}

export function disconnect(provider: PlatformId): Promise<void> {
  return invoke<void>(provider === 'kick' ? 'kick_disconnect' : 'youtube_disconnect');
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

/**
 * Ask each connected platform whether its session still works, signing out any
 * that have been revoked. Resolves to the provider ids that were signed out.
 */
export function validateSessions(): Promise<PlatformId[]> {
  return invoke<PlatformId[]>('validate_platform_sessions');
}
