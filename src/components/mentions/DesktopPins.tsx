// The pinned mentions of one desktop chat surface (SN-284): registers the
// surface with Rust, reads its answer and draws the block. Mounted by the chat
// widget in the pinned message's slot, so the pins and the channel's pin share
// one place.

import { useCallback, useEffect, useMemo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { MentionPinsBlock, type PinsForm } from './MentionPins';
import { roomRef, useMentionPins, usePinSurface } from '../../hooks/useMentionPins';
import { useDesktopChatShown } from '../../hooks/useChatSurfaceShown';
import { requestMentionFocus } from '../../stores/mentionFocusStore';
import { openProfilePopup } from '../../utils/openProfilePopup';
import { Logger } from '../../utils/logger';
import type { PinRoomRef } from '../../utils/pinSurfaces';
import type { MentionPin } from '../../types';
import type { ProviderId } from '../../types/providers';

/** The platforms a pin on a desktop surface can be opened for. */
const DESKTOP_OPENS = ['twitch', 'kick', 'youtube', 'tiktok'];

export interface DesktopPinsProps {
  /** `main-chat`, `dock`, `pane:<...>`, `main-provider`, `blended`. */
  surfaceId: string;
  /** The chats this surface shows. */
  rooms: PinRoomRef[];
  /** The pane the viewer is acting in. */
  active: boolean;
  chatViewActive: boolean;
  searchOpen: boolean;
  /** Rust's form for the pins card; `full` when the host has no stack layout. */
  form?: PinsForm;
  height?: number;
  moreCards?: number;
  /** A pin of this surface's own chat: scroll to it. */
  onOpenOwn: (pin: MentionPin) => void;
  onMeasure?: (m: { form: PinsForm; height: number }) => void;
  /** The answer, for a host that lays its stack out around the pins. */
  onPins?: (pins: readonly MentionPin[]) => void;
  className?: string;
}

/** Hooks half: the surface and its answer. */
export function useDesktopPins(p: Pick<DesktopPinsProps, 'surfaceId' | 'rooms' | 'active' | 'chatViewActive' | 'searchOpen'>) {
  const shown = useDesktopChatShown({ chatViewActive: p.chatViewActive, searchOpen: p.searchOpen });
  const roomsKey = JSON.stringify(p.rooms);
  const spec = useMemo(
    () => ({
      id: p.surfaceId,
      rooms: JSON.parse(roomsKey) as PinRoomRef[],
      active: p.active,
      shown,
      mode: '',
      strips_only: false,
      over_video: false,
      opens: DESKTOP_OPENS,
    }),
    [p.surfaceId, roomsKey, p.active, shown],
  );
  const { reportVisible } = usePinSurface(spec);
  const answer = useMentionPins(p.surfaceId);
  return { shown, reportVisible, ...answer };
}

/** A press on a pin's name opens that chat's card: the pin's own channel,
 *  never the surface's, and with no moderator context. */
export function openPinnedProfile(pin: MentionPin, e: React.MouseEvent): void {
  const row = pin.row;
  void openProfilePopup({
    userId: row.user_id,
    username: row.username,
    displayName: row.display_name,
    color: row.color ?? '',
    badges: [],
    channelId: row.tags?.['room-id'] ?? pin.entry.channel_id ?? '',
    channelName: pin.entry.channel_login || pin.channel_mark?.channel || '',
    isModerator: false,
    provider: (row.provider ?? 'twitch') as ProviderId,
    clientX: e.clientX,
    clientY: e.clientY,
  }).catch((err) => Logger.debug('[MentionPins] profile card failed:', err));
}

export function DesktopPins(props: DesktopPinsProps) {
  const { shown, reportVisible, pins } = useDesktopPins(props);
  const { onOpenOwn, onPins } = props;
  const openOther = useCallback((pin: MentionPin) => {
    // The inbox reads it when its row is opened, not here.
    requestMentionFocus({ id: pin.entry.id, room: pin.entry.room });
  }, []);
  const openOwn = useCallback(
    (pin: MentionPin) => {
      invoke('open_mention', { id: pin.entry.id }).catch((err) => {
        Logger.warn('[MentionPins] open_mention failed:', err);
      });
      onOpenOwn(pin);
    },
    [onOpenOwn],
  );
  useEffect(() => {
    onPins?.(pins);
  }, [pins, onPins]);
  if (pins.length === 0) return null;
  return (
    <MentionPinsBlock
      pins={pins}
      form={props.form ?? 'full'}
      height={props.height}
      moreCards={props.moreCards}
      shown={shown}
      reportVisible={reportVisible}
      onOpenOwn={openOwn}
      onOpenOther={openOther}
      onUsernameClick={openPinnedProfile}
      onMeasure={props.onMeasure}
      className={props.className ?? 'max-h-[40vh]'}
    />
  );
}

export { roomRef };
export default DesktopPins;
