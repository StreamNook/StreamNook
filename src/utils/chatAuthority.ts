// Which chat rows you are allowed to act on.
//
// The chat panel can show more than one platform's messages at once (a
// streamer's Twitch, Kick and YouTube chats merged into the one you are
// watching). Everything that MODERATES, however, takes its target from
// panel-level values that mean "the channel being watched": the broadcaster id,
// the moderator flag, the provider. Point any of those at a row from another
// platform and they still look like a valid request.
//
// The concrete failure this exists to stop: Twitch user ids and Kick user ids
// are both numeric strings, so banning a focused Kick row while watching Twitch
// sends the Kick user's id to Helix as a Twitch user id. Helix accepts it and
// bans a real, unrelated account, and the toast names the Kick chatter.
//
// So the rule is one line, and it is deliberately not "route each action to its
// own platform": that needs per-row broadcaster ids and per-row moderator
// authority threaded through three surfaces, and is its own feature.
//
//   Authority is home-only. A row from another platform is readable and
//   replyable, and invisible to every path that moderates, targets a
//   broadcaster, or infers your privileges.

import type { ProviderId } from '../types/providers';
import type { BackendChatMessage } from '../services/twitchChat';

/**
 * The platform a row came from.
 *
 * A raw IRC string is the home provider by definition: strings only ever reach
 * a slice from the channel's own paths — the Twitch read connection, an
 * optimistic copy of something you just sent, or an injected system row. Every
 * companion platform delivers structured objects that carry their own
 * `provider`.
 */
export function rowProvider(
  message: string | BackendChatMessage,
  homeProvider: ProviderId,
): ProviderId {
  if (typeof message === 'string') return homeProvider;
  return (message.provider as ProviderId | undefined) ?? 'twitch';
}

/**
 * Whether this row came from the platform being watched, and may therefore be
 * moderated with the panel's broadcaster id and moderator flag.
 */
export function isHomeRow(
  message: string | BackendChatMessage,
  homeProvider: ProviderId,
): boolean {
  return rowProvider(message, homeProvider) === homeProvider;
}
