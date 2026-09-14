// Mobile chat: ChatMessageList (the full cosmetics render path: paints, badges,
// atmosphere washes) + a lean touch composer, over N concurrent channels.
// Deliberately NOT ChatWidget: that component drags mod rooms and MultiNook into
// the bundle.
//
// Channels are owned by chatTabsStore, which holds a reference-counted
// subscription per tab, so every open room stays connected and switching tabs is
// a re-render rather than a reconnect. This pane renders whichever tab is active
// via `useChannelChat`.
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { CaretDown } from 'phosphor-react';
import ChatMessageList from '../../components/ChatMessageList';
import {
  setChannelPaused,
  useChannelChat,
  useChannelEmotes,
} from '../../stores/chatConnectionStore';
import { useAppStore } from '../../stores/AppStore';
import { useChatUserStore } from '../../stores/chatUserStore';
import { parseMessage } from '../../services/twitchChat';
import type { BackendChatMessage } from '../../services/twitchChat';
import { Logger } from '../../utils/logger';
import { MobileChatInput } from './MobileChatInput';
import { UserProfileSheet, type SheetUser } from '../profile/UserProfileSheet';
import { ChatTabStrip } from './ChatTabStrip';
import { AddChatSheet } from './AddChatSheet';
import { useChatTabsStore } from './chatTabsStore';
import { useChatWatchdog } from './useChatWatchdog';
import { banUser, deleteMessage, isModeratorFrom, pinMessage, unbanUser } from './modActions';
import { deriveChatGating } from './chatGating';
import { useFollowStatus } from './useFollowStatus';
import { ChatFanOut, type FanAction, type FanTarget } from './ChatFanOut';
import { useLongPressDrag } from './useLongPressDrag';
import { usePinStore } from '../../stores/pinStore';
import { formatDuration } from '../../utils/timeoutRamp';
import { scrollChatToMessage } from '../../utils/scrollChatToMessage';

// How long a pause/resume transition is protected from being reversed. Matches
// desktop's ChatWidget, and is what stops a touchmove and the compositor scroll
// it causes from fighting each other frame by frame. See `pause` below.
const PAUSE_SETTLE_MS = 120;

export const MobileChatPane: React.FC = () => {
  const currentStream = useAppStore((s) => s.currentStream);
  const currentUser = useAppStore((s) => s.currentUser);
  const addToast = useAppStore((s) => s.addToast);

  const tabs = useChatTabsStore((s) => s.tabs);
  const activeChannel = useChatTabsStore((s) => s.activeChannel);
  const syncStreamTab = useChatTabsStore((s) => s.syncStreamTab);
  const removeTab = useChatTabsStore((s) => s.removeTab);
  const reload = useChatTabsStore((s) => s.reload);

  // Keep the stream-following tab pointed at whatever is playing.
  useEffect(() => {
    syncStreamTab(
      currentStream?.user_login ?? null,
      currentStream?.user_id ?? null,
      currentStream?.user_name || currentStream?.user_login || '',
      currentStream?.profile_image_url ?? null,
    );
  }, [
    currentStream?.user_login,
    currentStream?.user_id,
    currentStream?.user_name,
    currentStream?.profile_image_url,
    syncStreamTab,
  ]);

  const activeTab = useMemo(
    () => tabs.find((t) => t.channel === activeChannel) ?? null,
    [tabs, activeChannel],
  );

  const chat = useChannelChat(activeChannel);
  const {
    messages,
    renderToken,
    deletedMessageIds,
    clearedUserContexts,
    userBadges,
    roomState,
    isConnected,
  } = chat;

  const emotes = useChannelEmotes(activeChannel, activeTab?.channelId ?? null, 'twitch');

  // Rebuilds the connection if it dies while the app is open. Every room rides
  // one connection, so watching the active one covers all of them.
  useChatWatchdog(activeChannel);

  const isModerator = isModeratorFrom(userBadges);
  /** Every Helix mod action keys off the channel's numeric id. */
  const broadcasterId = activeTab?.channelId ?? null;
  // Only asked when follower mode is actually on, so an unrestricted room costs
  // nothing. -1 is off; 0 and above are some flavour of followers-only.
  const isFollowing = useFollowStatus(broadcasterId, (roomState?.followersOnly ?? -1) >= 0);
  const gating = useMemo(
    () => deriveChatGating(roomState, userBadges, isFollowing),
    [roomState, userBadges, isFollowing],
  );
  // Moderator tools are ON wherever you have the badge. A mod opening their own
  // channel expects their tools, and making them hunt for a toggle every time is
  // the wrong default. The destructive actions are protected by the fan's
  // geometry instead: Ban sits on the far arc, so it takes a deliberate longer
  // reach than Delete.
  //
  // Stored as the set of channels where you turned them OFF, so the default
  // survives switching rooms without an effect resetting state. Losing the badge
  // disarms regardless, since `isModerator` gates it.
  const [modToolsOffFor, setModToolsOffFor] = useState<ReadonlySet<string>>(new Set());
  const modToolsOn = !!activeChannel && !modToolsOffFor.has(activeChannel);
  const modToolsArmed = isModerator && modToolsOn;

  const toggleModTools = useCallback(() => {
    if (!activeChannel) return;
    setModToolsOffFor((prev) => {
      const next = new Set(prev);
      if (next.has(activeChannel)) next.delete(activeChannel);
      else next.add(activeChannel);
      return next;
    });
  }, [activeChannel]);

  // Paused state and the reply draft are keyed BY CHANNEL rather than reset when
  // you switch tabs. Same outcome, but nothing has to write state from an effect
  // (which is a cascading-render error under react-hooks v7), and coming back to
  // a room restores nothing stale because the key simply stops matching.
  const [pausedChannel, setPausedChannel] = useState<string | null>(null);
  // Briefly marks the message a reply jumped to, so it is findable after the
  // glide lands. ChatMessageList already renders this; mobile just never set it.
  const [highlightedMessageId, setHighlightedMessageId] = useState<string | null>(null);
  const isPaused = !!activeChannel && pausedChannel === activeChannel;

  const [sheetUser, setSheetUser] = useState<SheetUser | null>(null);
  const [addChatOpen, setAddChatOpen] = useState(false);
  // Long-press arms the fan-out over the pressed row; it owns every message
  // action now, so there is no separate action sheet.
  const [fanTarget, setFanTarget] = useState<FanTarget | null>(null);
  const [replyDraft, setReplyDraft] = useState<{
    messageId: string;
    username: string;
    channel: string;
  } | null>(null);
  const replyTo = replyDraft && replyDraft.channel === activeChannel ? replyDraft : null;

  // Held still so the memoized children below actually stay memoized. This pane
  // re-renders on every arriving message, and an inline arrow prop is a new
  // reference each time, which defeats React.memo entirely. onCloseChat is the
  // easy one to miss: it is conditional, so it silently churns whenever the
  // active tab is a room you added by hand.
  const handleCancelReply = useCallback(() => setReplyDraft(null), []);
  const handleAddChat = useCallback(() => setAddChatOpen(true), []);
  const handleCloseAddChat = useCallback(() => setAddChatOpen(false), []);
  const handleCloseSheet = useCallback(() => setSheetUser(null), []);
  const handleReload = useCallback(() => {
    if (activeChannel) reload(activeChannel);
  }, [activeChannel, reload]);
  const activeTabChannel = activeTab?.channel;
  const activeTabPinned = activeTab?.pinnedToStream;
  const handleCloseChat = useMemo(
    () =>
      activeTabChannel && !activeTabPinned ? () => removeTab(activeTabChannel) : undefined,
    [activeTabChannel, activeTabPinned, removeTab],
  );

  // Dedupe set for the cosmetics registration loop, tagged with the channel it
  // describes so switching rooms invalidates it without an effect reset.
  const processed = useRef<{ channel: string | null; ids: Set<string> }>({
    channel: null,
    ids: new Set(),
  });

  // Seed YOUR OWN row on connect: Twitch never echoes your PRIVMSG back, so the
  // per-message loop below never adds you, and your paint/badge would have
  // nothing to attach to. Mirrors ChatWidget's own-user seeding.
  useEffect(() => {
    if (!isConnected || !currentUser?.user_id) return;
    useChatUserStore.getState().addUser({
      userId: currentUser.user_id,
      username: currentUser.login || currentUser.username || '',
      displayName: currentUser.display_name || currentUser.username || currentUser.user_id,
      color: '#9147FF',
    });
  }, [isConnected, currentUser?.user_id, currentUser]);

  // Register every chatter with chatUserStore: this is what triggers the
  // per-user cosmetics fetch (7TV paint/badge, third-party badges). Lean
  // Twitch-only mirror of ChatWidget's message-processing effect.
  useEffect(() => {
    const { addUser } = useChatUserStore.getState();
    if (processed.current.channel !== activeChannel) {
      processed.current = { channel: activeChannel, ids: new Set() };
    }
    const seen = processed.current.ids;
    const currentIds = new Set<string>();

    for (const message of messages) {
      let msgId: string | undefined;
      if (typeof message === 'string') {
        const m = message.match(/(?:^@|;)id=([^;\s]+)/);
        msgId = m ? m[1] : undefined;
      } else {
        msgId = (message as BackendChatMessage).id;
      }
      if (msgId) {
        currentIds.add(msgId);
        if (seen.has(msgId)) continue;
        seen.add(msgId);
      }

      try {
        let userId: string | undefined;
        let username: string | undefined;
        let displayName: string | undefined;
        let userColor: string | undefined;
        let channelId: string | undefined;

        if (typeof message === 'string') {
          const channelIdMatch = message.match(/room-id=([^;]+)/);
          channelId = channelIdMatch ? channelIdMatch[1] : undefined;
          const parsed = parseMessage(message, channelId);
          userId = parsed.tags.get('user-id');
          username = parsed.username;
          displayName = parsed.tags.get('display-name') || parsed.username;
          userColor = parsed.color;
        } else {
          const backend = message as BackendChatMessage;
          const parsed = parseMessage(backend);
          userId = backend.tags['user-id'] || backend.user_id;
          username = backend.username;
          displayName = backend.display_name || backend.username;
          userColor = backend.color || parsed.color;
          channelId = backend.tags['room-id'];
        }

        if (userId && username && displayName) {
          const ctxId = channelId || activeTab?.channelId || '';
          const ctxName = activeTab?.channel || '';
          addUser(
            { userId, username, displayName, color: userColor || '#9147FF' },
            ctxId ? { channelId: ctxId, channelName: ctxName } : undefined,
          );
        }
      } catch (err) {
        Logger.error('[MobileChatPane] failed to parse message:', err, message);
      }
    }

    for (const id of seen) {
      if (!currentIds.has(id)) seen.delete(id);
    }
  }, [messages, renderToken, activeChannel, activeTab?.channelId, activeTab?.channel]);

  const getMessageId = useCallback((message: string | BackendChatMessage): string | null => {
    if (typeof message !== 'string') return message.id;
    const idMatch = message.match(/(?:^|;)id=([^;]+)/);
    return idMatch ? idMatch[1] : null;
  }, []);

  // Pause transitions are GUARDED, and the guards are the whole fix for the
  // "chat thrashes up and down and takes a few swipes to register" bug.
  //
  // The loop that produced it: a 6px read-back drag fires onPauseIntent ->
  // pause(true); the compositor scrolls 6px; ChatMessageList's handleScroll sees
  // distance < 100, clears its own up-intent flag, and reports back; this
  // handler saw `distance < 24 && isPaused` and immediately un-paused; the
  // auto-scroll effect then re-ran with the intent flag already cleared and did
  // `scrollTop = scrollHeight` WITH THE FINGER STILL DOWN. Next touchmove, same
  // again. It only broke out when a single frame cleared the window, which is
  // exactly why a slow deliberate swipe never registered but a hard flick did.
  //
  // Desktop never had this because ChatWidget wraps the same transition in an
  // idempotence check and a settle window. Porting those two is what breaks the
  // oscillation: pause at t=0, the resume attempt ~5ms later is refused, and by
  // the time 120ms has passed a real swipe is well clear of the resume
  // threshold.
  //
  // NOTE we deliberately do NOT port desktop's 150px *pause* threshold. Desktop
  // pauses from scroll distance; mobile pauses from INTENT (the first downward
  // touchmove), which is the better trigger and the one that makes a slow
  // read-back pause on the first gesture. A 150px gate here would mean a
  // deliberate 60-80px read-back never paused at all.
  const isPausedRef = useRef(false);
  const lastPauseToggleRef = useRef(0);
  useEffect(() => {
    isPausedRef.current = isPaused;
  }, [isPaused]);

  const pause = useCallback(
    (p: boolean, opts?: { force?: boolean }) => {
      if (!activeChannel) return;
      // Idempotent. Without this, onPauseIntent fired pause(true) on EVERY touch
      // frame, and each call reached setChannelPaused -> bumpRevision -> a
      // zustand setState -> a full re-render of every chat subscriber. That was
      // a per-frame re-render storm on top of the visual glitch.
      if (isPausedRef.current === p) return;
      const now = Date.now();
      if (!opts?.force && now - lastPauseToggleRef.current < PAUSE_SETTLE_MS) return;
      lastPauseToggleRef.current = now;
      isPausedRef.current = p;
      setPausedChannel(p ? activeChannel : null);
      setChannelPaused(activeChannel, p);
    },
    [activeChannel],
  );

  // `isUserScroll` is computed by ChatMessageList (up-intent AND more than 50px
  // from the bottom) and was previously DISCARDED here, which is the other half
  // of why a compositor scroll could un-pause. Resume only on a real user scroll
  // that has actually arrived at the bottom; 30px matches desktop.
  const onScroll = useCallback(
    (distanceToBottom: number, isUserScroll?: boolean) => {
      if (!isPausedRef.current) return;
      if (isUserScroll === false) return;
      if (distanceToBottom < 30) pause(false);
    },
    [pause],
  );

  // Returning to live: unpause, then let the list glide smoothly to the bottom
  // (ChatMessageList exposes its eased scroll for exactly this).
  const resumeLive = useCallback(() => {
    // `force`: this is an explicit tap on the Resume control, so it must never
    // be swallowed by the settle window the way a stray scroll report should be.
    pause(false, { force: true });
    requestAnimationFrame(() => {
      (
        window as Window & typeof globalThis & { __chatScrollToBottom?: () => void }
      ).__chatScrollToBottom?.();
    });
  }, [pause]);

  // Tapping the "replying to" line jumps to the message being replied to.
  //
  // Everything except this handler already existed: ChatMessage has carried the
  // onClick since the desktop build, ChatMessageList forwards the callback, and
  // rows already carry data-message-id. Mobile was passing a no-op, so the line
  // looked tappable and did nothing.
  //
  // Pausing FIRST is load-bearing, not politeness. Auto-scroll re-pins the list
  // to the bottom on every incoming message, so in a busy channel an unpaused
  // jump gets undone before the glide finishes. `force` because a deliberate
  // navigation must beat the settle window that exists to swallow stray scroll
  // reports.
  const handleReplyClick = useCallback(
    (parentMsgId: string) => {
      pause(true, { force: true });
      const ok = scrollChatToMessage(parentMsgId, { align: 'center' });
      if (!ok) {
        addToast('Original message is no longer in chat history', 'info');
        return;
      }
      setHighlightedMessageId(parentMsgId);
      // Long enough to find the message by eye after the glide lands, short
      // enough that it does not linger as a permanent-looking selection.
      window.setTimeout(() => setHighlightedMessageId(null), 2000);
    },
    [pause, addToast],
  );

  const onUsernameClick = useCallback(
    (userId: string, username: string, displayName: string, color: string) => {
      setSheetUser({ userId, username, displayName, color });
    },
    [],
  );

  /** Pull the author and body out of a rendered row's message id. */
  const describeMessage = useCallback(
    (messageId: string) => {
      for (const message of messages) {
        if (getMessageId(message) !== messageId) continue;
        try {
          const parsed = parseMessage(message as string | BackendChatMessage);
          const username = parsed.username ?? '';
          if (!username) return null;
          return {
            messageId,
            username,
            userId: parsed.tags.get('user-id') ?? '',
            content: parsed.content ?? '',
          };
        } catch {
          return null;
        }
      }
      return null;
    },
    [messages, getMessageId],
  );

  // Suppress the synthesized contextmenu entirely: the fan-out owns the hold now,
  // and letting both fire would open a menu behind the fan.
  const onListContextMenu = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
  }, []);

  const listRef = useRef<HTMLDivElement | null>(null);

  const resolveRow = useCallback(
    (el: HTMLElement) => el.getAttribute('data-message-id'),
    [],
  );

  const onArmFan = useCallback(
    (messageId: string, x: number, y: number) => {
      const found = describeMessage(messageId);
      if (!found) return;
      setFanTarget({ ...found, originX: x, originY: y });
    },
    [describeMessage],
  );

  const press = useLongPressDrag({
    resolve: resolveRow,
    onArm: onArmFan,
    scrollLockRef: listRef,
  });

  const closeFan = useCallback(() => {
    setFanTarget(null);
    press.release();
  }, [press]);

  const runFanAction = useCallback(
    (action: FanAction, timeoutSecs?: number) => {
      const t = fanTarget;
      closeFan();
      if (!t) return;
      switch (action) {
        case 'reply':
          setReplyDraft({
            messageId: t.messageId,
            username: t.username,
            channel: activeChannel ?? '',
          });
          break;
        case 'copy':
          void navigator.clipboard.writeText(t.content).catch(() => {});
          break;
        case 'profile':
          setSheetUser({
            userId: t.userId,
            username: t.username,
            displayName: t.username,
            color: '#9147FF',
          });
          break;
        case 'delete':
          if (broadcasterId) {
            void deleteMessage(broadcasterId, t.messageId).then(
              (ok) => !ok && addToast('Could not delete that message.', 'error'),
            );
          }
          break;
        case 'timeout':
          if (broadcasterId && t.userId) {
            void banUser(broadcasterId, t.userId, timeoutSecs ?? 600).then((ok) => {
              if (!ok) addToast('Could not time out that user.', 'error');
              else
                addToast(
                  `Timed out ${t.username} for ${formatDuration(timeoutSecs ?? 600)}`,
                  'success',
                  {
                    label: 'Undo',
                    onClick: () => void unbanUser(broadcasterId, t.userId),
                  },
                );
            });
          }
          break;
        case 'ban':
          if (broadcasterId && t.userId) {
            void banUser(broadcasterId, t.userId, null).then((ok) => {
              if (!ok) addToast('Could not ban that user.', 'error');
              else
                addToast(`Banned ${t.username}`, 'success', {
                  label: 'Undo',
                  onClick: () => void unbanUser(broadcasterId, t.userId),
                });
            });
          }
          break;
        case 'pin':
          if (broadcasterId) {
            void pinMessage(broadcasterId, t.messageId).then((ok) => {
              if (ok) usePinStore.getState().requestRefresh();
              else addToast('Could not pin that message.', 'error');
            });
          }
          break;
      }
    },
    [fanTarget, closeFan, activeChannel, broadcasterId, addToast],
  );

  // Tap-to-reply, but only while paused. A live list scrolls under your finger,
  // so a tap on a moving row is a coin flip; pausing makes the target stable.
  // Paused rows get a visible reply affordance (see ChatMessageList styling via
  // the data attribute below) so the gesture is discoverable rather than secret.
  const onListClick = useCallback(
    (e: React.MouseEvent) => {
      if (!isPaused || press.isArmed()) return;
      const el = e.target as HTMLElement;
      // Usernames and links keep their own behaviour.
      if (el.closest('a,[data-no-drag],button')) return;
      const row = el.closest('[data-message-id]');
      if (!row) return;
      const messageId = row.getAttribute('data-message-id');
      if (!messageId) return;
      const found = describeMessage(messageId);
      if (!found) return;
      setReplyDraft({
        messageId: found.messageId,
        username: found.username,
        channel: activeChannel ?? '',
      });
    },
    [isPaused, press, describeMessage, activeChannel],
  );


  return (
    <div className="flex-1 min-h-0 flex flex-col">
      <ChatTabStrip />
      <div
        ref={listRef}
        // `select-none` is load-bearing, not tidiness: Android WebView answers a
        // long-press on text by starting its OWN text selection, which wins the
        // gesture and the fan never arms. Chat text needs no selection anyway —
        // the fan's Copy bucket is the way to take a message.
        className="flex-1 min-h-0 relative select-none [-webkit-touch-callout:none]"
        onContextMenu={onListContextMenu}
        onPointerDown={press.onPointerDown}
        onPointerMove={press.onPointerMove}
        onPointerUp={press.onPointerUp}
        onPointerCancel={press.onPointerCancel}
        onClick={onListClick}
        // While paused, rows are tappable to reply. Signposted by the hint pill
        // below rather than left as a secret gesture.
        data-tap-reply={isPaused ? 'true' : undefined}
      >
        <ChatMessageList
          messages={messages}
          renderToken={renderToken}
          isPaused={isPaused}
          onPauseIntent={() => pause(true)}
          onScroll={onScroll}
          onUsernameClick={onUsernameClick}
          onReplyClick={handleReplyClick}
          onEmoteRightClick={() => {}}
          onUsernameRightClick={() => {}}
          onBadgeClick={() => {}}
          highlightedMessageId={highlightedMessageId}
          deletedMessageIds={deletedMessageIds}
          clearedUserContexts={clearedUserContexts}
          emotes={emotes}
          getMessageId={getMessageId}
        />
        {isPaused && (
          <div className="absolute bottom-2 left-0 right-0 flex flex-col items-center gap-1 z-10 pointer-events-none">
            <span className="glass-badge rounded-full px-2 py-0.5 text-[10.5px] text-textMuted">
              Tap a message to reply
            </span>
            <button
              onClick={resumeLive}
              className="pointer-events-auto glass-button flex items-center gap-1 whitespace-nowrap rounded-full px-3 py-1 text-[12px] font-medium text-textPrimary"
            >
              <CaretDown size={11} weight="bold" />
              Resume live
            </button>
          </div>
        )}
      </div>
      <MobileChatInput
        channel={activeChannel}
        channelId={broadcasterId}
        channelLabel={activeTab?.label ?? null}
        gating={gating}
        emotes={emotes}
        replyTo={replyTo}
        onCancelReply={handleCancelReply}
        isModerator={isModerator}
        modToolsOn={modToolsOn}
        onToggleModTools={toggleModTools}
        onAddChat={handleAddChat}
        onReload={handleReload}
        onCloseChat={handleCloseChat}
      />
      {/* Channel context matters: Twitch badges like sub tiers and moderator are
          scoped to the room, so without it the profile shows only global ones. */}
      <UserProfileSheet
        user={sheetUser}
        channelId={broadcasterId}
        channelName={activeChannel}
        onClose={handleCloseSheet}
      />
      <AddChatSheet open={addChatOpen} onClose={handleCloseAddChat} />
      {/* Long-press fan-out. Owns every per-message action now, for everyone:
          moderators simply get more buckets. */}
      <ChatFanOut
        target={fanTarget}
        isModerator={modToolsArmed}
        canPin={modToolsArmed && !!broadcasterId}
        onCommit={runFanAction}
        onCancel={closeFan}
      />
    </div>
  );
};
