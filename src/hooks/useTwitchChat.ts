// Backwards-compatible thin wrapper around the per-channel chatConnectionStore.
//
// Historically this file owned ~1200 lines of WebSocket bridge code, message
// routing, optimistic-send replacement, reconnect/health-check logic, and per-
// channel cache state — all scoped to a single "current channel." That logic
// now lives in `src/stores/chatConnectionStore.ts` and supports concurrent
// channels (one IRC connection, N reference-counted subscribers).
//
// This hook preserves the exact return shape ChatWidget consumes so the main
// app continues to work without any caller-side changes:
//
//   const { messages, connectChat, sendMessage, isConnected, error, setPaused,
//           deletedMessageIds, clearedUserContexts, roomState, userBadges }
//     = useTwitchChat();
//
// New code (MultiChat tabs, etc.) should call the store directly via
// `useChannelChat(channel)` + `acquireChannel` / `releaseChannel`.

import { useCallback, useEffect, useState } from 'react';
import {
  acquireChannel,
  releaseChannel,
  sendChannelMessage,
  setChannelPaused,
  useChannelChatMeta,
  type RoomState,
  type SendUserInfo,
  type SendAsAccount,
} from '../stores/chatConnectionStore';
import { Logger } from '../utils/logger';

// Re-export the moderation type so existing imports (`useTwitchChat`-relative)
// keep working: ChatWidget pulls this from various places.
export type { ModerationContext, ClearedUserEntry, RoomState } from '../stores/chatConnectionStore';

// C7 checkpoint C: the snapshot fields (messages, renderToken, deletion
// marks, liveMessageCount) left this shape - the self-subscribing
// ChatMessagesPanel and PausedNewCount own them now, so the widget's parent
// no longer re-renders per chat flush. This hook returns actions + the
// low-frequency meta the chrome renders.
export interface UseTwitchChatReturn {
  connectChat: (channel: string, roomId?: string) => Promise<void>;
  // `userInfo` is optional only so this shares a call signature with the
  // provider send path (which needs no Twitch identity). The Twitch send below
  // still requires it and bails without it, so no caller behavior changes.
  sendMessage: (
    messageText: string,
    userInfo?: SendUserInfo,
    replyParentMsgId?: string,
    senderAccount?: SendAsAccount | null,
  ) => Promise<void>;
  isConnected: boolean;
  error: string | null;
  setPaused: (paused: boolean) => void;
  roomState: RoomState;
  userBadges: string | null;
}

/**
 * The channel one chat surface holds, and the release that balances it.
 *
 * The channel is owned BEFORE its acquire is awaited. It used to be recorded
 * only once the acquire resolved, so an unmount while the first connect was
 * still in flight (closing a stream or an offline room right after opening
 * it) released nothing: the slice leaked at one reference, and the next open
 * of that channel took the store's "+1 ref" path and never connected.
 *
 * Owning it early means the unmount of StrictMode's replayed mount (setup,
 * cleanup, setup on the same instance, in dev) now really lets go, and the
 * widget never asks for the channel a second time. `resume` takes back what
 * that cleanup let go.
 */
export function createChannelOwner() {
  let current: string | null = null;
  let currentRoomId: string | null = null;
  let parked: { channel: string; roomId: string | null } | null = null;
  const owner = {
    current: (): string | null => current,
    async connect(channel: string, roomId?: string): Promise<void> {
      const target = channel.toLowerCase();
      parked = null;
      const previous = current;
      if (previous === target) {
        // Same channel: refresh the room id only if a new one was provided.
        // Acquiring again is idempotent (just bumps the ref count) but also
        // updates the channelId if it was null before. Pair it with a release
        // so the ref count stays balanced.
        if (roomId) {
          currentRoomId = roomId;
          await acquireChannel(target, roomId);
          await releaseChannel(target);
        }
        return;
      }
      // Switching channels: acquire the new one first (so the bridge stays up
      // if it's the same Rust connection), then release the previous.
      current = target;
      currentRoomId = roomId ?? null;
      try {
        await acquireChannel(target, roomId ?? null);
      } finally {
        if (previous) await releaseChannel(previous);
      }
    },
    dispose(): void {
      const channel = current;
      current = null;
      if (!channel) return;
      parked = { channel, roomId: currentRoomId };
      Logger.debug(`[useTwitchChat] Unmount: releasing ${channel}`);
      void releaseChannel(channel);
    },
    resume(): Promise<void> | null {
      const held = parked;
      parked = null;
      if (!held || current) return null;
      return owner.connect(held.channel, held.roomId ?? undefined);
    },
  };
  return owner;
}

export const useTwitchChat = (): UseTwitchChatReturn => {
  // Which channel this hook instance owns. The shim acquires on connectChat()
  // and releases on unmount or on switching channels, routed through the
  // ref-counted store so multiple consumers (MultiChat + main app) share one
  // underlying IRC connection.
  const [owner] = useState(createChannelOwner);
  // Render-side mirror of the owned channel: the meta selector needs it during
  // render. Set as soon as a switch starts, so a connect that fails shows its
  // error on the channel it was for.
  const [activeChannel, setActiveChannel] = useState<string | null>(null);

  const meta = useChannelChatMeta(activeChannel);

  const connectChat = useCallback(
    async (channel: string, roomId?: string) => {
      const targetKey = channel.toLowerCase();
      if (owner.current() !== targetKey) setActiveChannel(targetKey);
      try {
        await owner.connect(targetKey, roomId);
      } catch (err) {
        Logger.error('[useTwitchChat] connectChat failed:', err);
        throw err;
      }
    },
    [owner],
  );

  const sendMessage = useCallback(
    async (
      messageText: string,
      userInfo?: SendUserInfo,
      replyParentMsgId?: string,
      senderAccount?: SendAsAccount | null,
    ) => {
      const channel = owner.current();
      if (!channel) {
        Logger.warn('[useTwitchChat] sendMessage called with no active channel');
        return;
      }
      // A Twitch send needs the sender identity for the optimistic echo, so a
      // missing one is a caller bug rather than something to paper over.
      if (!userInfo) {
        Logger.warn('[useTwitchChat] sendMessage called without user info');
        return;
      }
      await sendChannelMessage(channel, messageText, userInfo, replyParentMsgId, senderAccount);
    },
    [owner],
  );

  const setPaused = useCallback(
    (paused: boolean) => {
      const channel = owner.current();
      if (!channel) return;
      setChannelPaused(channel, paused);
    },
    [owner],
  );

  // Release on unmount so the store's ref count drops cleanly. StrictMode
  // replays a mount on this same instance: the replayed setup takes back what
  // its cleanup let go, since the widget will not ask for the channel again.
  useEffect(() => {
    void owner.resume()?.catch((err: unknown) => Logger.error('[useTwitchChat] resume failed:', err));
    return () => owner.dispose();
  }, [owner]);

  return {
    connectChat,
    sendMessage,
    isConnected: meta.isConnected,
    error: meta.error,
    setPaused,
    roomState: meta.roomState,
    userBadges: meta.userBadges,
  };
};
