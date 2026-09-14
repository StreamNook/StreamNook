import type { ProviderId } from '../types/providers';

/**
 * The platform id of the chatter who sent a message.
 *
 * Twitch carries it as the IRC `user-id` tag. Kick and YouTube carry no such tag
 * — their id rides on the message struct as `providerUserId` (`kick.rs` stamps
 * only `display-name` and `id`; `youtube.rs` adds `avatar` and `msg-id`). Every
 * username-click site used to read the tag directly and bail when it was absent,
 * which is why clicking a Kick or YouTube chatter did nothing at all: not the
 * wrong card, no card.
 *
 * Returns the RAW platform id, not a composite key — callers that need to
 * namespace it (cosmetics, per-user overrides, message history) build the key
 * themselves with `makeKey`, because some of them still want the bare Twitch id.
 */
export interface ParsedChatterSource {
  provider?: ProviderId;
  tags: Map<string, string>;
  providerUserId?: string;
}

export function chatterId(parsed: ParsedChatterSource): string | undefined {
  const provider = parsed.provider ?? 'twitch';
  const id = provider === 'twitch' ? parsed.tags.get('user-id') : parsed.providerUserId;
  return id || undefined;
}

/** The platform a message came from, defaulting to Twitch for bare rows. */
export function chatterProvider(parsed: ParsedChatterSource): ProviderId {
  return parsed.provider ?? 'twitch';
}

/**
 * The key a chatter's PERSISTED message history is stored under in Rust.
 *
 * Twitch keeps the bare user id so existing entries keep resolving; every other
 * platform is namespaced, because the id spaces overlap and Kick user 676 is not
 * Twitch user 676. `irc_service::history_key_for` builds the same string on the
 * write side, so the two must be changed together.
 */
export function historyKey(userId: string, provider: ProviderId = 'twitch'): string {
  return provider === 'twitch' ? userId : `${provider}:${userId}`;
}
