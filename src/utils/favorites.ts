// What identifies a FAVOURITE, and how to dedupe a list of them.
//
// A favourite is a personal watchlist entry: it works whether or not you follow
// the channel, which means the backend has to be able to ask the platform "is
// this one live" from the stored key alone. So the key is not "whatever names
// this row" — it is the identifier that platform's live check actually accepts,
// and that differs per platform. See `favoriteIdOf`.

import type { FavoriteChannel, TwitchStream } from '../types';
import type { ProviderId } from '../types/providers';
import { makeKey } from './providerKey.ts';
import { streamProvider } from './streamProvider.ts';

/**
 * The key a favourite is stored under, or `null` when the row carries no stable
 * channel identity (see YouTube below) and the caller must resolve one first.
 *
 * Deliberately NOT `followIdentifier` from `streamProvider`. That helper reads
 * "non-Twitch and has a user_id, so use the user_id", which is right for
 * YouTube and wrong everywhere else:
 *
 *   twitch   numeric `user_id`   Helix answers `streams?user_id=`, and every
 *                                favourite that predates multi-platform support
 *                                is already stored this way.
 *   kick     slug (`user_login`) Kick's live check queries `slug=`. Kick rows
 *                                DO carry a numeric `user_id`, and keying on it
 *                                would produce a favourite that never resolves.
 *   youtube  UC id (`user_id`)   `user_login` on a browse row is the VIDEO id.
 *                                A video id names one broadcast and never comes
 *                                back, so a favourite keyed by one would sit
 *                                there resolving a finished stream forever.
 *   tiktok   @handle             Stored so you can return to the channel;
 *                                TikTok ships no live check, so it is never
 *                                reported live.
 */
export function favoriteIdOf(
  stream: Pick<TwitchStream, 'provider' | 'user_id' | 'user_login'> | null | undefined,
): string | null {
  if (!stream) return null;
  const provider = streamProvider(stream);

  if (provider === 'twitch') {
    return stream.user_id || null;
  }

  if (provider === 'youtube') {
    // A UC id is the channel. A handle also addresses one, so it is an
    // acceptable fallback; a bare video id is not, and returning null here is
    // what makes the caller resolve rather than persist something broken.
    const uc = stream.user_id;
    if (uc && uc.startsWith('UC')) return makeKey('youtube', uc);
    const login = stream.user_login;
    if (login && login.startsWith('@')) return makeKey('youtube', login);
    return null;
  }

  return stream.user_login ? makeKey(provider, stream.user_login) : null;
}

/**
 * A favourite id the OLD sidebar wrote as a raw `stream.user_id`.
 *
 * On a YouTube row that id is the channel's `UC…`, stored with no provider
 * prefix, so `parseKey` reads it back as a Twitch login: the heart never fills
 * again, the channel shows up in no list, and the backend sweep hands the UC id
 * to Helix as a Twitch user id. Found in real settings data.
 *
 * A 24-character id starting `UC` is YouTube's own channel-id shape (the same
 * test `first_channel_id` applies in `youtube_media.rs`), and a Twitch id is
 * always numeric, so this cannot mistake one for the other.
 */
export function isStrayYouTubeFavoriteId(id: string): boolean {
  return id.length === 24 && id.startsWith('UC');
}

/** The identity sidecar row for a stream, captured at favourite time so the
 *  channel can still be drawn (a name and a face) once it goes offline. */
export function favoriteMetaOf(stream: TwitchStream, id: string): FavoriteChannel {
  const provider = streamProvider(stream);
  return {
    id,
    provider,
    // The identifier chat and playback address, which for YouTube is the
    // channel and NOT the video the row happened to arrive as.
    channel: channelOf(stream, provider),
    display_name: stream.user_name || undefined,
    avatar: stream.profile_image_url || undefined,
    added_at: new Date().toISOString(),
  };
}

function channelOf(stream: TwitchStream, provider: ProviderId): string {
  if (provider === 'twitch') return stream.user_login;
  if (provider === 'youtube') {
    if (stream.user_id?.startsWith('UC')) return stream.user_id;
    return stream.user_login;
  }
  return stream.user_login;
}

/**
 * Collapse rows that name the same channel, keeping the first of each.
 *
 * Must NOT be `streamKey`: favourites are merged from three live sources at
 * once (Twitch follows, the provider follow poller, the favourites sweep), and
 * on YouTube the same channel arrives keyed by video id from a browse row and
 * by UC id from a live check. A `streamKey` dedupe lets both through and the
 * channel renders twice.
 *
 * Rows with no resolvable identity fall back to their own key rather than being
 * dropped: a row we can't name is still a row worth showing.
 */
export function dedupeByFavoriteId<T extends TwitchStream>(streams: T[]): T[] {
  const seen = new Set<string>();
  const out: T[] = [];
  for (const s of streams) {
    const key = favoriteIdOf(s) ?? `${streamProvider(s)}:${s.user_login?.toLowerCase() ?? ''}`;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(s);
  }
  return out;
}
