import type { ProviderId } from '../types/providers';
import { DEFAULT_PROVIDER, isProviderId } from '../types/providers.ts';

// Composite source key "<provider>:<channel>" used across the chat store, the
// activity store, and the add-source flow. Mirrors the Rust codec in
// services/providers/key.rs. A bare key (no recognised provider prefix) is
// treated as a legacy Twitch login so older persisted state keeps working.

/** Platforms whose channel identifier is CASE-SENSITIVE and must never be
 *  normalised. A YouTube id addresses a specific video — `AGr94tpNVkw` and
 *  `agr94tpnvkw` are different things, and lowercasing one yields "This video is
 *  unavailable". Twitch logins and Kick slugs are case-insensitive, so those keep
 *  being lowercased and their keys stay byte-identical to before. */
const CASE_SENSITIVE: ProviderId[] = ['youtube'];

export function normalizeChannel(provider: ProviderId, channel: string): string {
  const c = channel.trim();
  return CASE_SENSITIVE.includes(provider) ? c : c.toLowerCase();
}

export function makeKey(provider: ProviderId, channel: string): string {
  return `${provider}:${normalizeChannel(provider, channel)}`;
}

/** The key a (provider, channel) pair's chat slice is STORED under.
 *
 *  Exported because anything addressing a slice from outside — a merged feed's
 *  per-source revision sum, a pause fanned out across sources — has to fold the
 *  same way. Opening this derivation a second time is how a lookup starts
 *  silently missing its own slice: `makeKey` preserves case for YouTube while
 *  storage lowercases unconditionally. */
export function sliceLookupKey(provider: ProviderId, channel: string): string {
  return (provider === 'twitch' ? channel : makeKey(provider, channel)).toLowerCase();
}

export interface ParsedKey {
  provider: ProviderId;
  channel: string;
}

export function parseKey(key: string): ParsedKey {
  const i = key.indexOf(':');
  if (i !== -1) {
    const maybe = key.slice(0, i);
    if (isProviderId(maybe)) {
      return { provider: maybe, channel: key.slice(i + 1) };
    }
  }
  // Bare login, or text that merely contains a colon: read as Twitch.
  return { provider: DEFAULT_PROVIDER, channel: key.toLowerCase() };
}
