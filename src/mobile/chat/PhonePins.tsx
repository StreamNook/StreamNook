// The pinned mentions of one phone chat surface (SN-284): the watch screen's
// chat or the Chats tab. Registers the surface with Rust (one per screen,
// never one per chat tab, so a tab switch changes `rooms`, not the id), reads
// its answer and draws the block on `chrome-glaze`. Over the video (landscape
// overlay, the tall layout's lower third) it wears the over-video ink.

import { useCallback, useMemo } from 'react';
import { MentionPinsBlock, type PinsForm } from '../../components/mentions/MentionPins';
import { roomRef, useMentionPins, usePinSurface } from '../../hooks/useMentionPins';
import { usePageVisible } from '../../hooks/useChatSurfaceShown';
import { chatSurfaceShown } from '../../utils/chatSurfaceShown';
import { requestMentionFocus } from '../../stores/mentionFocusStore';
import { useMobileNavStore, backTakenBeforePlayer } from '../navStore';
import { useMentionsStore, openPinnedMention, roomName, roomProvider } from '../mentions/mentionInbox';
import type { InboxSurface } from '../mentions/mentionInbox';
import type { MentionPin } from '../../types';

/** The platforms a tap on a pin can open on the phone. */
const PHONE_OPENS = ['twitch', 'kick', 'youtube', 'tiktok'];

export type PhonePinSurface = 'phone-watch' | 'phone-chats';

export interface PhonePinsInput {
  surfaceId: PhonePinSurface;
  /** The chat on screen as Rust's room key (`xqc`, `kick:slug`), or null. */
  room: string | null;
  /** Watch: `stacked`, `landscape`, `vertical`, `over-video`. */
  mode: string;
  /** The chat is out of sight: the mini box, PiP, landscape with chat away,
   *  the Chats tab behind the player or another tab. */
  chatOutOfSight: boolean;
  stripsOnly?: boolean;
  overVideo?: boolean;
}

/** Whether any sheet or full-screen layer sits over the chat. */
export function useAnySheetOpen(): boolean {
  const navCovered = useMobileNavStore(backTakenBeforePlayer);
  const inboxSheet = useMentionsStore((s) => s.sheetIn !== null);
  return navCovered || inboxSheet;
}

export function usePhonePins(input: PhonePinsInput) {
  const pageVisible = usePageVisible();
  const sheetOpen = useAnySheetOpen();
  const shown = chatSurfaceShown({
    platform: 'android',
    pageVisible,
    chatOutOfSight: input.chatOutOfSight || !input.room,
    sheetOpen,
  });
  const rooms = useMemo(() => {
    if (!input.room) return [];
    const r = roomRef(roomProvider(input.room), roomName(input.room));
    return r ? [r] : [];
  }, [input.room]);
  const spec = useMemo(
    () => ({
      id: input.surfaceId,
      rooms,
      active: !input.chatOutOfSight,
      shown,
      mode: input.mode,
      strips_only: input.stripsOnly ?? false,
      over_video: input.overVideo ?? false,
      opens: PHONE_OPENS,
    }),
    [input.surfaceId, rooms, input.chatOutOfSight, shown, input.mode, input.stripsOnly, input.overVideo],
  );
  const { reportVisible } = usePinSurface(spec);
  const answer = useMentionPins(input.surfaceId);
  return { shown, reportVisible, ...answer };
}

/** A pin from another chat opens the Mentions sheet at that entry. */
export function openPinInInbox(pin: MentionPin, surface: InboxSurface): void {
  useMentionsStore.setState({ sheetIn: surface, arrival: null });
  requestMentionFocus({ id: pin.entry.id, room: pin.entry.room });
}

export function PhonePins({
  pins,
  shown,
  reportVisible,
  form = 'full',
  height,
  moreCards,
  inboxSurface,
  overVideo = false,
  onBeforeOpen,
  onMeasure,
}: {
  pins: readonly MentionPin[];
  shown: boolean;
  reportVisible: (ids: string[]) => void;
  form?: PinsForm;
  height?: number;
  moreCards?: number;
  inboxSurface: InboxSurface;
  overVideo?: boolean;
  /** The lower third grows the chat first. */
  onBeforeOpen?: () => void;
  onMeasure?: (m: { form: PinsForm; height: number }) => void;
}) {
  const openOwn = useCallback(
    (pin: MentionPin) => {
      onBeforeOpen?.();
      openPinnedMention(pin.entry);
    },
    [onBeforeOpen],
  );
  const openOther = useCallback(
    (pin: MentionPin) => {
      onBeforeOpen?.();
      openPinInInbox(pin, inboxSurface);
    },
    [inboxSurface, onBeforeOpen],
  );
  if (pins.length === 0) return null;
  return (
    <MentionPinsBlock
      pins={pins}
      form={form}
      height={height}
      moreCards={moreCards}
      shown={shown}
      phone
      overVideo={overVideo}
      reportVisible={reportVisible}
      onOpenOwn={openOwn}
      onOpenOther={openOther}
      onMeasure={onMeasure}
      className="max-h-[40vh]"
    />
  );
}
