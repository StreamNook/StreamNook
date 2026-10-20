// The player band while a Twitch channel is offline: the desktop's offline
// card in its compact form (who, when they were last live, what comes next,
// who is waiting), wired to the same store. Rust's watch session starts the
// stream in place when the channel goes live, so there is nothing to retry.
// No latest-broadcast button: the phone's player chrome is a live player's.
//
// Stacked above chat, the band shows the offline art alone and the card sits
// in a chin directly below it (MobileOfflineChin), so no text covers the art.
// Where there is no room below (landscape), the card stays over the art.
//
// In the mini box (and system picture-in-picture) the card has no room, so the
// box shows only who it is and that they are offline, rather than black.
import React, { useState } from 'react';
import { useShallow } from 'zustand/react/shallow';
import { useAppStore } from '../../stores/AppStore';
import { OfflineCard } from '../../components/OfflineRoomScreen';
import { makeKey } from '../../utils/providerKey';
import { streamProvider } from '../../utils/streamProvider';
import { useChannelSheet } from '../watch/MobileChannelSheet';

function useOfflineRoomCard() {
  return useAppStore(
    useShallow((s) => ({
      room: s.offlineRoom,
      stream: s.currentStream,
      live: s.offlineRoomLive,
      joining: s.offlineRoomJoining,
      startStream: s.startStream,
      playVod: s.playOfflineRoomVod,
    })),
  );
}

export const MobileOfflineRoom: React.FC<{
  mini?: boolean;
  /** The card is drawn below the band (MobileOfflineChin): the art only. */
  cardBelow?: boolean;
}> = ({ mini = false, cardBelow = false }) => {
  const { room, stream, live, joining, startStream, playVod } = useOfflineRoomCard();
  const name = room?.display_name || stream?.user_name || stream?.user_login || 'This channel';
  const avatar = room?.avatar_url || stream?.profile_image_url || null;
  if (mini) return <MiniRoom name={name} avatar={avatar} />;
  return (
    <OfflineCard
      room={room}
      fallbackName={name}
      fallbackAvatar={stream?.profile_image_url}
      live={live}
      joining={joining}
      onPlayVod={() => void playVod()}
      onVideos={stream ? () => useChannelSheet.getState().open(stream) : undefined}
      onWatchLive={(l) => void startStream(l.user_login, l, true)}
      compact
      part={cardBelow ? 'art' : 'all'}
    />
  );
};

/** The offline room's card, in flow directly below the band, for as long as
 *  the band shows the offline art (not while a playback notice holds it). */
export const MobileOfflineChin: React.FC = () => {
  const { room, stream, live, joining, startStream, playVod } = useOfflineRoomCard();
  const offline = useAppStore((s) => s.streamUrl === 'offline');
  const notice = useAppStore((s) => s.playbackNotice);
  const noticeShown =
    !!stream && notice?.key === makeKey(streamProvider(stream), stream.user_login);
  if (!offline || noticeShown) return null;
  const name = room?.display_name || stream?.user_name || stream?.user_login || 'This channel';
  // The gap below keeps chat's faded top row clear of the card's edge, so the
  // card reads as sitting above chat, not over it.
  return (
    <div className="shrink-0 px-4 pt-3 pb-2">
      <OfflineCard
        room={room}
        fallbackName={name}
        fallbackAvatar={stream?.profile_image_url}
        live={live}
        joining={joining}
        onPlayVod={() => void playVod()}
        onVideos={stream ? () => useChannelSheet.getState().open(stream) : undefined}
        onWatchLive={(l) => void startStream(l.user_login, l, true)}
        compact
        part="card"
      />
    </div>
  );
};

/** The face with a quiet ring (no live red) and, under it, a hollow ring and
 *  "Offline". Nothing over the picture; the box itself is the tap target. */
const MiniRoom: React.FC<{ name: string; avatar: string | null }> = ({ name, avatar }) => {
  // A face that fails to load falls back to the first letter, never a broken
  // image glyph in a box this small.
  const [broken, setBroken] = useState<string | null>(null);
  const showImage = !!avatar && broken !== avatar;
  return (
    <div
      className="absolute inset-0 flex flex-col items-center justify-center gap-1.5 pointer-events-none"
      style={{ background: 'linear-gradient(to bottom, var(--color-surface), #000)' }}
      role="img"
      aria-label={`${name} is offline`}
    >
      {showImage ? (
        <img
          src={avatar}
          alt=""
          draggable={false}
          onError={() => setBroken(avatar)}
          className="w-8 h-8 rounded-full object-cover ring-2 ring-white/25"
        />
      ) : (
        <span className="w-8 h-8 rounded-full bg-white/15 ring-2 ring-white/25 flex items-center justify-center text-[13px] font-bold text-white">
          {name.charAt(0).toUpperCase()}
        </span>
      )}
      <span className="flex items-center gap-[5px] text-[12px] font-semibold leading-none text-white/80">
        <span className="w-[7px] h-[7px] rounded-full border-[1.5px] border-white/60" />
        Offline
      </span>
    </div>
  );
};
