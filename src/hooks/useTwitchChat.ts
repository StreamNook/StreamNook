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

import { useCallback, useEffect, useRef, useState } from 'react';
import {
  acquireChannel,
  releaseChannel,
  sendChannelMessage,
  setChannelPaused,
  useChannelChatMeta,
  type ClearedUserEntry,
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

export const useTwitchChat = (): UseTwitchChatReturn => {
  // Track which channel this hook instance currently owns. The shim acquires
  // on connectChat() and releases on unmount or on switching channels — same
  // lifecycle the prior single-instance hook had, but now routed through the
  // ref-counted store so multiple consumers (MultiChat + main app) share one
  // underlying IRC connection.
  const currentChannelRef = useRef<string | null>(null);
  // Render-side mirror of the ref: the meta selector needs the channel during
  // render, and a ref must not be read there. Set alongside the ref on every
  // successful switch.
  const [activeChannel, setActiveChannel] = useState<string | null>(null);

  const meta = useChannelChatMeta(activeChannel);

  const connectChat = useCallback(async (channel: string, roomId?: string) => {
    const targetKey = channel.toLowerCase();
    const previous = currentChannelRef.current;

    if (previous === targetKey) {
      // Same channel — refresh room-id only if a new one was provided.
      if (roomId) {
        // Acquiring again is idempotent (just bumps the ref count) but also
        // updates the channelId if it was null before. Pair it with a release
        // so the ref count stays balanced.
        await acquireChannel(targetKey, roomId);
        await releaseChannel(targetKey);
      }
      return;
    }

    // Switching channels: acquire the new one first (so the bridge stays up
    // if it's the same Rust connection) then release the previous.
    try {
      await acquireChannel(targetKey, roomId ?? null);
      currentChannelRef.current = targetKey;
      setActiveChannel(targetKey);
      if (previous) {
        await releaseChannel(previous);
      }
    } catch (err) {
      Logger.error('[useTwitchChat] connectChat failed:', err);
      // If acquire failed, don't leak the previous reference.
      throw err;
    }
  }, []);

  const sendMessage = useCallback(
    async (
      messageText: string,
      userInfo?: SendUserInfo,
      replyParentMsgId?: string,
      senderAccount?: SendAsAccount | null,
    ) => {
      const channel = currentChannelRef.current;
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
    [],
  );

  const setPaused = useCallback((paused: boolean) => {
    const channel = currentChannelRef.current;
    if (!channel) return;
    setChannelPaused(channel, paused);
  }, []);

  // Release on unmount so the store's ref count drops cleanly.
  useEffect(() => {
    return () => {
      const channel = currentChannelRef.current;
      if (channel) {
        Logger.debug(`[useTwitchChat] Unmount: releasing ${channel}`);
        void releaseChannel(channel);
        currentChannelRef.current = null;
      }
    };
  }, []);

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
