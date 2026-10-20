// Who the channel on screen is streaming with (a Stream Together group) or
// sharing its chat with (a Shared Chat session), as a bottom sheet: the desktop
// Together popover's panel, where tapping a member switches to their stream.
// The phone has no MultiNook grid, so the panel leaves those actions out.
import React from 'react';
import type { TwitchStream } from '../../types';
import type { ChannelGroup } from '../../utils/sharedViewers';
import { useAppStore } from '../../stores/AppStore';
import { TogetherPanel } from '../../components/SharedViewers';
import { MobileSheet } from '../ui/MobileSheet';

/** What the group already knows about a member, as the start's seed: the
 *  header shows their name, face and viewers on the tap, and Rust fills in the
 *  title, category and start time. Never a made-up start. */
function memberSeed(collab: ChannelGroup, login: string): TwitchStream | undefined {
  const m = collab.members.find((x) => x.login.toLowerCase() === login.toLowerCase());
  if (!m) return undefined;
  return {
    id: '',
    user_id: m.user_id,
    user_login: m.login,
    user_name: m.display_name || m.login,
    title: '',
    viewer_count: m.viewer_count,
    game_name: '',
    thumbnail_url: '',
    started_at: '',
    profile_image_url: m.avatar_url ?? undefined,
  };
}

export const TogetherSheet: React.FC<{ collab: ChannelGroup | null; open: boolean; onClose: () => void }> = ({
  collab,
  open,
  onClose,
}) => (
  <MobileSheet open={open && collab !== null} onClose={onClose}>
    {collab && (
      <TogetherPanel
        collab={collab}
        onOpenChannel={(login) => void useAppStore.getState().startStream(login, memberSeed(collab, login))}
        onDone={onClose}
      />
    )}
  </MobileSheet>
);
