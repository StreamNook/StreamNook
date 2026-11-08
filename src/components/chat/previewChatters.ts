import { createContext } from 'react';
import type { ChatUser } from '../../stores/chatUserStore';

/**
 * Made-up chatters for a settings preview that draws REAL chat rows: their
 * paint, 7TV badge, add-on badges and atmosphere, keyed by the row's
 * cosmetics key (the Twitch user id). A row inside the provider reads its
 * sender here before the chat-user store, so a preview can show every cosmetic
 * setting without putting fake names into the store real chat completes and
 * suggests from. Real chat never provides it.
 */
export type PreviewChatter = Pick<
  ChatUser,
  'paint' | 'seventvBadge' | 'thirdPartyBadges' | 'atmosphereId' | 'cologne' | 'themeHiddenInChat'
> & {
  /** A StreamNook member, so the row draws the member badge (with the default
   *  mark) as a member's row does. Anyone wearing an atmosphere is one. */
  streamNookMember?: boolean;
};

export const PreviewChattersContext = createContext<ReadonlyMap<string, PreviewChatter> | null>(null);
