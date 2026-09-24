import type { PlatformId } from '../services/platformAccountService';

/**
 * A stream the platform serves only to a signed-in account (a TikTok LIVE
 * that is 18+). `start_stream` rejects with a plain string for every other
 * failure, and with `{ sign_in: { provider, channel, message } }` for this one,
 * so the player can offer the sign-in where the stream failed.
 */
export interface SignInRequired {
  provider: PlatformId;
  channel: string;
  /** Complete on its own; shown as is. */
  message: string;
}

const PLATFORMS: readonly string[] = ['kick', 'youtube', 'tiktok'];

export function signInRequiredFrom(e: unknown): SignInRequired | null {
  if (!e || typeof e !== 'object' || !('sign_in' in e)) return null;
  const s = (e as { sign_in: unknown }).sign_in;
  if (!s || typeof s !== 'object') return null;
  const { provider, channel, message } = s as Record<string, unknown>;
  if (typeof provider !== 'string' || !PLATFORMS.includes(provider)) return null;
  if (typeof channel !== 'string' || typeof message !== 'string' || !message) return null;
  return { provider: provider as PlatformId, channel, message };
}
