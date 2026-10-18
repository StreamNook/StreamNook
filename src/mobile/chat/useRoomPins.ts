// Pinned messages for the chat on screen, under the player and on the Chats
// screen alike.
//
// Pinned messages come from Rust's channel-state service, the same source as
// the desktop chat header and the iPhone: it polls each watched room's pins
// and pushes `channel-state` only when they change, and its pin and unpin
// commands keep re-reading until Twitch reflects the action.
//
// Every open Twitch room is watched, not just the one on screen, so switching
// tabs paints that room's pin at once instead of after a fetch. The watches
// are diffed against the tab list rather than torn down and rebuilt, because
// dropping a room's last watch drops its state and the banner would blink on
// every tab added. Rust refcounts watches, so two surfaces showing chat at
// once (the mini player over the Chats screen) is fine.
import { useEffect, useMemo, useRef } from 'react';
import { usePinStore } from '../../stores/pinStore';
import {
  refreshChannelState,
  unwatchChannel,
  useChannelState,
  watchChannel,
} from '../../stores/channelStateStore';
import { useChatTabsStore } from './chatTabsStore';

export interface PinnedMessage {
  id: string;
  message_id: string;
  message_text: string;
  sender_id: string;
  sender_name: string;
  sender_color: string;
  sender_badges: Array<{ set_id: string; version: string }>;
}

// One empty list, so a room with no pins keeps a stable identity for the
// effects that read it.
const NO_PINS: PinnedMessage[] = [];

/** The pins of the room on screen; `active` is false while no chat shows. */
export function useRoomPins(active: boolean): PinnedMessage[] {
  const refreshNonce = usePinStore((s) => s.refreshNonce);
  const twitchRooms = useChatTabsStore((s) =>
    s.tabs
      .filter((t) => t.provider === 'twitch' && t.channelId)
      .map((t) => `${t.login.toLowerCase()} ${t.channelId}`)
      .join('\n'),
  );
  const roomWatches = useRef(new Map<string, string>());
  useEffect(() => {
    const held = roomWatches.current;
    const want = new Map<string, string>();
    if (active && twitchRooms) {
      for (const row of twitchRooms.split('\n')) {
        const [login, id] = row.split(' ');
        want.set(login, id);
      }
    }
    for (const login of [...held.keys()]) {
      if (want.has(login)) continue;
      held.delete(login);
      void unwatchChannel(login);
    }
    for (const [login, id] of want) {
      if (held.has(login)) continue;
      held.set(login, id);
      void watchChannel(login, id);
    }
  }, [active, twitchRooms]);
  useEffect(() => {
    const held = roomWatches.current;
    return () => {
      for (const login of held.keys()) void unwatchChannel(login);
      held.clear();
    };
  }, []);

  // Keyed to the CHAT on screen, not to the stream. A pinned message is a
  // property of the room you are reading, and the state is looked up by that
  // room's own login, so one channel's pin can never sit above another's
  // messages. Kick rooms have no Twitch id and show none.
  const pinLogin = useChatTabsStore((s) => {
    const t = s.tabs.find((tab) => tab.channel === s.activeChannel);
    return t?.provider === 'twitch' && t.channelId ? t.login.toLowerCase() : null;
  });
  const pinState = useChannelState(pinLogin);
  const pinned = useMemo(
    () => (pinState?.pinned as PinnedMessage[] | undefined) ?? NO_PINS,
    [pinState?.pinned],
  );
  // The chat rows read this to flip a Pin control into Unpin.
  useEffect(() => {
    usePinStore.getState().setPinnedIds(pinned.map((m) => m.message_id).filter(Boolean));
  }, [pinned]);
  // A pin or unpin from this phone asks for the room's pins at once. Rust's pin
  // commands already settle on their own; this covers anything that bumps the
  // nonce without going through them. Compared against the last nonce seen so
  // switching rooms does not refetch.
  const pinNonceSeen = useRef(refreshNonce);
  useEffect(() => {
    if (refreshNonce === pinNonceSeen.current) return;
    pinNonceSeen.current = refreshNonce;
    if (pinLogin) void refreshChannelState(pinLogin, 'pinned');
  }, [refreshNonce, pinLogin]);
  return pinned;
}
