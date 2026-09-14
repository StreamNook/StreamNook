// Chat for a non-Twitch stream watched in the MAIN window.
//
// Non-Twitch sources render through the SAME ChatWidget as Twitch, addressed by
// a `channelOverride` — the seam MultiChat's `ProviderViaChatWidget` already
// uses, so every Twitch-only behavior inside ChatWidget stays gated on
// `provider` with no new branches.
//
// Unlike the MultiChat pane, this one needs no metadata polling of its own:
// AppStore resolved the channel when the stream started and refreshes viewers
// and title from the same live check that detects the stream ending, so the
// override is derived straight from `currentStream`.
//
// It also serves the MultiNook grid, whose active chat can be any tile rather
// than the solo stream. That caller passes `details` from the slot; everything
// else, including the ChatWidget seam, is identical.

import { useMemo } from 'react';
import ChatWidget, { type ChatWidgetChannelOverride } from './ChatWidget';
import { useAppStore } from '../stores/AppStore';
import type { ProviderId } from '../types/providers';

/** Display fields for a channel that is NOT the app's current solo stream.
 *  A MultiNook tile is exactly that case: the grid's active chat can be any
 *  tile, while `currentStream` still describes whatever the solo player last
 *  had (or nothing). */
export interface ProviderChatDetails {
  user_id?: string;
  user_name?: string;
  title?: string;
  game_name?: string;
  viewer_count?: number;
  started_at?: string;
  profile_image_url?: string;
}

interface MainProviderChatProps {
  provider: ProviderId;
  channel: string;
  /** Absent means derive from `currentStream`, the solo case, byte-identical to
   *  before this prop existed. */
  details?: ProviderChatDetails;
}

export default function MainProviderChat({ provider, channel, details }: MainProviderChatProps) {
  const currentStream = useAppStore((s) => s.currentStream);
  // `details` wins when given: reading currentStream for a grid tile would
  // describe the wrong channel entirely.
  const src: ProviderChatDetails | null = details ?? currentStream ?? null;

  const channelOverride = useMemo<ChatWidgetChannelOverride>(
    () => ({
      provider,
      context: 'main',
      user_login: channel.toLowerCase(),
      user_id: src?.user_id ?? '',
      user_name: src?.user_name || channel,
      title: src?.title || undefined,
      game_name: src?.game_name || undefined,
      viewer_count: src?.viewer_count,
      started_at: src?.started_at || undefined,
      profile_image_url: src?.profile_image_url || undefined,
      is_live: currentStream?.is_live ?? true,
      // The main window always owns its moderation hotkeys; MultiChat scopes
      // them to the focused pane, which is why this flag exists at all.
      is_active: true,
    }),
    [provider, channel, src, currentStream],
  );

  return <ChatWidget channelOverride={channelOverride} />;
}
