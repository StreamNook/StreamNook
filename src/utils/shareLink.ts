// The link a share button copies or hands to the share sheet.
//
// Rust builds it (services/share_link.rs), for every platform: a
// streamnook.app/w/... web link that opens the stream in StreamNook when it is
// installed and on the platform's own page when it is not. The same module
// reads those links back when one opens the app.

import { invoke } from '@tauri-apps/api/core';
import type { TwitchStream } from '../types';

/** The share link for a stream row (or anything naming a channel the same way). */
export function shareLinkFor(
  stream: Pick<TwitchStream, 'provider' | 'user_login'> & { id?: string },
): Promise<string> {
  return invoke<string>('share_link', {
    provider: stream.provider ?? 'twitch',
    channel: stream.user_login ?? '',
    streamId: stream.id || null,
  });
}

/** The share link for a category. Only its id and name are sent. */
export function categoryShareLinkFor({ id, name }: { id: string; name: string }): Promise<string> {
  return invoke<string>('category_share_link', { id, name });
}

/**
 * Build a category's share link and hand it to `writeText`. True only when both
 * steps worked; any failure (a refused id, a blocked clipboard) is `false`, and
 * nothing here ever rejects, so a button needs no catch of its own.
 */
export async function copyCategoryShareLink(
  category: { id: string; name: string },
  writeText: (text: string) => Promise<void>,
): Promise<boolean> {
  try {
    await writeText(await categoryShareLinkFor(category));
    return true;
  } catch {
    return false;
  }
}
