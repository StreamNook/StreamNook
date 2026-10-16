// The player band while a Twitch channel is offline: the desktop's offline
// card in its compact form (who, when they were last live, what comes next,
// who is waiting), wired to the same store. Rust's watch session starts the
// stream in place when the channel goes live, so there is nothing to retry.
// No latest-broadcast button: the phone's player chrome is a live player's.
import React from 'react';
import { useShallow } from 'zustand/react/shallow';
import { useAppStore } from '../../stores/AppStore';
import { OfflineCard } from '../../components/OfflineRoomScreen';

export const MobileOfflineRoom: React.FC = () => {
  const { room, stream, live, joining, startStream } = useAppStore(
    useShallow((s) => ({
      room: s.offlineRoom,
      stream: s.currentStream,
      live: s.offlineRoomLive,
      joining: s.offlineRoomJoining,
      startStream: s.startStream,
    })),
  );
  return (
    <OfflineCard
      room={room}
      fallbackName={stream?.user_name || stream?.user_login || 'This channel'}
      fallbackAvatar={stream?.profile_image_url}
      live={live}
      joining={joining}
      onWatchLive={(l) => void startStream(l.user_login, l, true)}
      compact
    />
  );
};
