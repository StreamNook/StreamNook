// What identifies a FAVOURITE, and how to dedupe a list of them.
//
// A favourite is a personal watchlist entry: it works whether or not you follow
// the channel, which means the backend has to be able to ask the platform "is
// this one live" from the stored key alone. So the key is not "whatever names
// this row" — it is the identifier that platform's live check actually accepts,
// and that differs per platform. See `favoriteIdOf`.

import type { TwitchStream } from '../types';
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
 *
 * Rust twin: `favorite_id` in src-tauri/src/services/providers/key.rs. The two
 * must agree exactly: Rust drops a live favourite from the unified Discover
 * list by that one while the Favourites section is chosen by this one, so a
 * disagreement shows a channel twice or not at all. Change both together; both
 * read the `favorite_id` cases in providerTwins.fixtures.json, and so does the
 * iPhone's twin.
 */
export function favoriteIdOf(
  stream: Pick<TwitchStream, 'provider' | 'user_id' | 'user_login'> | null | undefined,
): string | null {
  if (!stream) return null;
  const provider = streamProvider(stream);

  const userId = stripOwnPrefix(provider, stream.user_id ?? '');
  const login = stripOwnPrefix(provider, stream.user_login ?? '');

  if (provider === 'twitch') {
    return userId || null;
  }

  if (provider === 'youtube') {
    // A UC id is the channel. A handle also addresses one, so it is an
    // acceptable fallback; a bare video id is not, and returning null here is
    // what makes Rust resolve the channel rather than persist something broken.
    if (userId.startsWith('UC')) return makeKey('youtube', userId);
    if (login.startsWith('@')) return makeKey('youtube', login);
    return null;
  }

  return login ? makeKey(provider, login) : null;
}

/** `id` trimmed and without leading copies of its own provider's prefix
 *  (`kick:xqc` on Kick is `xqc`), compared without case. Rust twin:
 *  `strip_own_prefix` in key.rs. */
function stripOwnPrefix(provider: string, id: string): string {
  const prefix = `${provider}:`;
  let rest = id.trim();
  while (rest.length > prefix.length && rest.slice(0, prefix.length).toLowerCase() === prefix) {
    rest = rest.slice(prefix.length);
  }
  return rest;
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
