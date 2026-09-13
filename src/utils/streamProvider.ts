// Helpers for reading the platform off a stream row.
//
// A row with no `provider` is Twitch — the same "bare = twitch" convention the
// composite-key codec uses (utils/providerKey), so every pre-existing producer
// (Helix responses, persisted state) keeps working untouched.

import type { TwitchStream } from '../types';
import type { ProviderId } from '../types/providers';
// Value imports carry the .ts extension (allowed by tsconfig's
// allowImportingTsExtensions) so `node --test` can run the unit tests directly,
// matching the convention the other tested utils use.
import { DEFAULT_PROVIDER, providerLabel } from '../types/providers.ts';
import { makeKey } from './providerKey.ts';

export function streamProvider(stream: Pick<TwitchStream, 'provider'> | null | undefined): ProviderId {
  return stream?.provider ?? DEFAULT_PROVIDER;
}

/**
 * The identifier a FOLLOW is keyed by — the channel's identity, not whatever
 * addresses the broadcast you are looking at.
 *
 * These differ on YouTube. A browse or search row is addressed by VIDEO id,
 * because that is what resolves to this particular stream, and `user_login`
 * carries it. But a video id names one broadcast and never comes back once it
 * ends, so following it would follow a stream rather than a channel — the row
 * would sit in the Following list resolving a finished broadcast forever. The
 * channel id (`user_id`, a `UC…`) is the stable identity, and it is what the
 * subscriptions import stores, so following by it is also what makes the two
 * lists agree.
 *
 * Twitch and Kick have no such split: the login / slug IS the channel — so
 * only YouTube takes the `user_id` branch. The old `!isTwitchStream` guard
 * swallowed Kick too, keying its follows by the NUMERIC user id: an entry the
 * live check (which queries `slug=`) could never resolve and the followed
 * check (which compares logins) could never read back. Mixing the two key
 * spaces has caused this class of bug repeatedly; keep them distinct.
 */
export function followIdentifier(
  stream: Pick<TwitchStream, 'provider' | 'user_login' | 'user_id'>,
): string {
  if (streamProvider(stream) === 'youtube' && stream.user_id) return stream.user_id;
  return stream.user_login;
}

export function isTwitchStream(stream: Pick<TwitchStream, 'provider'> | null | undefined): boolean {
  return streamProvider(stream) === 'twitch';
}

/**
 * The composite `<provider>:<channel>` key for a row. Twitch rows return a
 * BARE login (not `twitch:name`) so existing persisted keys — favorites,
 * resume snapshots, chat slices — stay byte-identical.
 */
export function streamKey(stream: Pick<TwitchStream, 'provider' | 'user_login'>): string {
  const provider = streamProvider(stream);
  // Case is normalised PER PLATFORM: lowercasing here would break a YouTube
  // video id, which is case-sensitive. Twitch keeps its bare lowercase login.
  return provider === 'twitch'
    ? stream.user_login.toLowerCase()
    : makeKey(provider, stream.user_login);
}

/**
 * Fill a thumbnail template. Twitch serves `{width}`/`%{width}` placeholders;
 * provider URLs are literal and pass through unchanged.
 */
export function streamThumbnail(stream: Pick<TwitchStream, 'thumbnail_url'>, width: number, height: number): string {
  const url = stream.thumbnail_url || '';
  if (!url.includes('{width}')) return url;
  return url
    .replace('%{width}', String(width))
    .replace('%{height}', String(height))
    .replace('{width}', String(width))
    .replace('{height}', String(height));
}

/**
 * The canonical platform watch URL for a channel. This is what `start_stream`
 * dispatches on, and what "Open on <platform>" links to.
 */
export function buildProviderUrl(provider: ProviderId, channel: string): string {
  switch (provider) {
    case 'kick':
      return `https://kick.com/${encodeURIComponent(channel)}`;
    case 'youtube':
      // @handles and UC ids address a channel's live page; a bare video id is a watch URL.
      if (channel.startsWith('@')) return `https://www.youtube.com/${channel}/live`;
      if (channel.startsWith('UC')) return `https://www.youtube.com/channel/${channel}/live`;
      return `https://www.youtube.com/watch?v=${encodeURIComponent(channel)}`;
    case 'tiktok':
      return `https://www.tiktok.com/@${encodeURIComponent(channel)}/live`;
    default:
      return `https://twitch.tv/${encodeURIComponent(channel)}`;
  }
}

/** "Open on Kick" / "Open on YouTube" menu label. */
export function openOnLabel(provider: ProviderId): string {
  return `Open on ${providerLabel(provider)}`;
}
