// The network step some channel inputs need before an add box stores them.
//
// Nearly everything the parsers in utils/parseChannelInput return is final. Two
// inputs are not, because the text alone can't say which channel is meant:
//
//   - A legacy YouTube /c/NAME or /user/NAME link. Those names predate handles
//     and are not handles (`/user/MrBeast6000` is MrBeast, while `@MrBeast6000`
//     does not exist), so Rust asks YouTube which channel the link points at,
//     the way youtube.com follows it, and the channel's UC id is stored.
//   - A Kick name with `_` or `-`. Kick's API answers only the exact slug, and
//     whether a username's underscores became hyphens in its slug depends on
//     the account, so Rust tries both spellings and stores the one Kick reports.
//
// It also holds the one lookup needed after a channel is stored: the name of a
// YouTube channel kept as a UC id, which otherwise reads as a string of letters.

import { invoke } from '@tauri-apps/api/core';
import { isYouTubeLegacyPath, kickSlugHasTwoSpellings } from '../utils/parseChannelInput';

/** The identifier to store for a parsed YouTube identifier. Rejects with a line
 *  for the add box when YouTube has no such channel or can't be reached. */
export async function resolveYouTubeIdentifier(id: string): Promise<string> {
  if (!isYouTubeLegacyPath(id)) return id;
  return invoke<string>('resolve_youtube_legacy_channel', { path: id });
}

/** The slug to store for a cleaned Kick name (kickSlugFromInput). Rejects with a
 *  line for the add box when Kick has no channel under either spelling; when
 *  Kick can't be reached, the name comes back as it was. */
export async function resolveKickSlug(slug: string): Promise<string> {
  if (!kickSlugHasTwoSpellings(slug)) return slug;
  return invoke<string>('resolve_kick_slug', { name: slug });
}

const youTubeTitles = new Map<string, Promise<string | null>>();

/** A YouTube channel's name for its `UC…` id, or null when YouTube won't say.
 *  Unlike a pane's live metadata this answers for an offline channel too, so a
 *  channel added by id doesn't have to go live before it reads as a name. One
 *  request per id per session; a failed one is forgotten so a later call retries. */
export function youTubeChannelTitle(channelId: string): Promise<string | null> {
  let title = youTubeTitles.get(channelId);
  if (!title) {
    title = invoke<{ title?: string | null }>('youtube_user_profile', { channelId })
      .then((profile) => profile?.title?.trim() || null)
      .catch(() => {
        youTubeTitles.delete(channelId);
        return null;
      });
    youTubeTitles.set(channelId, title);
  }
  return title;
}

/** A lookup's rejection as a line for the add box. Rust words its own errors for
 *  this; anything else (a failed IPC call) gets a generic line. */
export function lookupError(err: unknown, platform: string): string {
  return typeof err === 'string' && err ? err : `Couldn't look that ${platform} channel up. Please try again.`;
}
