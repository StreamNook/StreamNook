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
  return CASE_SENSITIVE.includes(provider) ? channel : channel.toLowerCase();
}

export function makeKey(provider: ProviderId, channel: string): string {
  return `${provider}:${normalizeChannel(provider, channel)}`;
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

export function keyProvider(key: string): ProviderId {
  return parseKey(key).provider;
}

export function keyChannel(key: string): string {
  return parseKey(key).channel;
}
