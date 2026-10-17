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

/** A StreamNook member's public profile page on the web. */
export function buildProfileUrl(login: string): string {
  return `https://streamnook.app/u/${encodeURIComponent((login ?? '').trim().toLowerCase())}`;
}
