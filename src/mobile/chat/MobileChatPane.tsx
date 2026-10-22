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
import { CaretDown, HandPointing, X } from 'phosphor-react';
import ChatMessageList from '../../components/ChatMessageList';
import {
  setChannelPaused,
  useChannelChat,
  useChannelChatMeta,
  useChannelEmotes,
} from '../../stores/chatConnectionStore';
import { useAppStore } from '../../stores/AppStore';
import { useChatUserStore } from '../../stores/chatUserStore';
import { parseMessage } from '../../services/twitchChat';
import type { BackendChatMessage } from '../../services/twitchChat';
import { Logger } from '../../utils/logger';
import { moderateWithToast } from '../../services/moderation';
import { MobileChatInput, type ChatGating } from './MobileChatInput';
import { UserProfileSheet, type SheetUser } from '../profile/UserProfileSheet';
import { ChatTabStrip } from './ChatTabStrip';
import { AddChatSheet } from './AddChatSheet';
import { useChatTabsStore } from './chatTabsStore';
import { useChatWatchdog } from './useChatWatchdog';
import { isModeratorFrom, pinMessage } from './modActions';
import { ChatFanOut, type FanAction, type FanTarget } from './ChatFanOut';
import { hapticCommit } from '../ui/haptics';
import { usePhonePrefs } from '../phonePrefs';
import { isWindowHidden } from '../../utils/windowVisibility';
import { useLongPressDrag } from './useLongPressDrag';
import { usePinStore } from '../../stores/pinStore';
import { scrollChatToMessage } from '../../utils/scrollChatToMessage';
import { useMentionJump } from '../mentions/useMentionJump';
import { useMentionsStore, type InboxSurface } from '../mentions/mentionInbox';
import { InboxSheet } from '../mentions/InboxSheet';
import { InboxArrival } from '../mentions/InboxArrival';
import { useMobileNavStore } from '../navStore';
import { useAutomodStore } from '../../stores/automodStore';
import { heldRowMessage, heldRows, isHeldRowId, mergeHeld } from './heldRows';
import { SwipeModerator } from './SwipeModerator';
import { messageReach, reachCanDelete, reachCanTimeout, type MessageReach, type SwipeTarget, type TouchSlider } from './swipeSteps';
import { invoke } from '@tauri-apps/api/core';
import { setVodReplayShowing, useVodReplaySnapshot, useVodReplayStore, type ReplayShowing } from '../../stores/vodReplayStore';
import { SegmentedControl, type Segment } from '../ui/SegmentedControl';
import { HoldWhile } from '../ui/HoldWhile';

// How long a pause/resume transition is protected from being reversed. Matches
// desktop's ChatWidget, and is what stops a touchmove and the compositor scroll
// it causes from fighting each other frame by frame. See `pause` below.
const PAUSE_SETTLE_MS = 120;
/** Set once a swipe has run (or its hint was dismissed): the hint stays gone. */
const SWIPE_HINT_KEY = 'sn-swipe-mod-learned';

/** What the read-only overlay leaves out of the fan. Module-level so the fan
 *  sees one array, not a fresh one per render. */
const READ_ONLY_HIDES: FanAction[] = ['reply'];
/** A non-Twitch room has no profile the phone can show: the sheet is Helix-backed. */
const OTHER_PLATFORM_HIDES: FanAction[] = ['profile'];
const OTHER_PLATFORM_READ_ONLY_HIDES: FanAction[] = ['reply', 'profile'];
/** What Hold menu buttons off takes out of the fan (Settings > Moderation); the
 *  user card's Moderate row and the swipe keep them. */
const MOD_TILES: FanAction[] = ['delete', 'timeout', 'ban'];

/** The desktop's VOD chat switch, word for word. */
const REPLAY_SEGMENTS: Segment<ReplayShowing>[] = [
  { value: 'replay', label: 'VOD Chat' },
  { value: 'live', label: 'Live Chat' },
];

/** The mention that buzzed last. The watch layer keeps its pane mounted under
 *  the mini player while the Chats screen shows another, so one mention reaches
 *  two panes; it buzzes once. */
let lastBuzzed: string | null = null;

/**
 * `readOnly`: the landscape overlay. Chat is for reading there; the composer,
 * tap-to-reply and the fan's Reply are gone, because a keyboard over a
 * sideways video is not something anyone asked for.
 *
 * `strip`: the tab strip over the list. The Chats screen draws its own, with
 * the play and add controls beside the tabs.
 *
 * `paused`: the pane is mounted but out of sight (the Chats tab behind another
 * tab). It reads no room, so arriving messages render nothing until it shows.
 *
 * `top`: what sits between the tab strip and the list (the watch screen's
 * poll, prediction and drop stack). In the flow, so it takes room from the
 * list rather than painting over it, and under the tabs, since it belongs to
 * the room on screen.
 *
 * `surface`: which chat this is (under the player, or the Chats tab), for the
 * inbox: its sheet opens in the chat whose tray was tapped, and a new
 * mention's card rises only over the chat that is on screen.
 *
 * The stream tab is pointed at the player by `useChatsLifecycle`, once, in the
 * shell: a pane only exists while it is on screen.
 */
export const MobileChatPane: React.FC<{
  readOnly?: boolean;
  strip?: boolean;
  paused?: boolean;
  /** Out of sight for now (under the mini player, in PiP, the app in the
   *  background): the room stays joined and sheets stay open, the list holds
   *  still and catches up in one render when seen again. */
  idle?: boolean;
  top?: React.ReactNode;
  surface?: InboxSurface;
}> = ({ readOnly = false, strip = true, paused = false, idle = false, top, surface = 'watch' }) => {
  const currentUser = useAppStore((s) => s.currentUser);
  const addToast = useAppStore((s) => s.addToast);

  const tabs = useChatTabsStore((s) => s.tabs);
  const activeChannel = useChatTabsStore((s) => s.activeChannel);
  const removeTab = useChatTabsStore((s) => s.removeTab);
  const reload = useChatTabsStore((s) => s.reload);

  const activeTab = useMemo(
    () => tabs.find((t) => t.channel === activeChannel) ?? null,
    [tabs, activeChannel],
  );

  // A VOD's recorded chat takes the stream's tab under the player, replayed by
  // Rust in step with the player (as on desktop and the iPhone), with the
  // switch back to the channel's live chat. Replay is read-only.
  const currentMediaType = useAppStore((s) => s.currentMediaType);
  const replayActive = useVodReplayStore((s) => s.active);
  const replayShowing = useVodReplayStore((s) => s.showing);
  const replayError = useVodReplayStore((s) => s.error);
  const vodHere =
    surface === 'watch' &&
    replayActive &&
    !!activeTab?.pinnedToStream &&
    (currentMediaType === 'video' || currentMediaType === 'clip' || currentMediaType === 'offline_chat');
  const replayOn = vodHere && replayShowing === 'replay';
  const noSend = readOnly || replayOn;
  const replayChat = useVodReplaySnapshot();
  const liveChat = useChannelChat(paused ? null : activeChannel);
  const chat = replayOn ? replayChat : liveChat;
  // The join's history answered with nothing: the room says so until a row arrives.
  const historyLoaded = useChannelChatMeta(activeChannel).historyLoaded;
  const {
    messages,
    renderToken,
    deletedMessageIds,
    clearedUserContexts,
    userBadges,
    roomState,
    isConnected,
  } = chat;

  const activeProvider = activeTab?.provider ?? 'twitch';
  const isTwitchRoom = activeProvider === 'twitch';
  const emotes = useChannelEmotes(activeTab?.login ?? null, activeTab?.channelId ?? null, activeProvider);

  // A mention buzzes once. The flash the row paints is easy to miss on a
  // phone held at arm's length, or with the screen dimmed for a long watch.
  // Rust stamps is_mentioned on every message, so this only has to notice
  // NEW rows: the ref remembers the newest id it has seen, and the first pass
  // for a room (or a reload's backfill) only primes it. Rows older than a few
  // seconds are history catching up, never a live mention.
  const mentionHaptic = usePhonePrefs((s) => s.mentionHaptic);
  const mentionCursor = useRef<{ channel: string | null; lastId: string | null }>({
    channel: null,
    lastId: null,
  });
  useEffect(() => {
    const cur = mentionCursor.current;
    const last = messages[messages.length - 1] as BackendChatMessage | string | undefined;
    const lastId = last && typeof last !== 'string' ? last.id : null;
    if (cur.channel !== activeChannel || cur.lastId === null) {
      mentionCursor.current = { channel: activeChannel, lastId };
      return;
    }
    if (lastId === cur.lastId) return;
    let buzz: string | null = null;
    const now = Date.now();
    for (let i = messages.length - 1; i >= 0; i--) {
      const m = messages[i] as BackendChatMessage | string;
      if (typeof m === 'string') continue;
      if (m.id === cur.lastId) break;
      if (!m.metadata?.is_mentioned) continue;
      if (m.user_id && m.user_id === currentUser?.user_id) continue;
      const ts = Number(m.timestamp);
      const at = Number.isFinite(ts) ? (ts < 1e12 ? ts * 1000 : ts) : Date.parse(m.timestamp);
      if (Number.isFinite(at) && now - at > 8000) continue;
      buzz = m.id;
      break;
    }
    mentionCursor.current = { channel: activeChannel, lastId };
    if (buzz && buzz !== lastBuzzed && mentionHaptic && !isWindowHidden()) {
      lastBuzzed = buzz;
      hapticCommit();
    }
  }, [messages, activeChannel, mentionHaptic, currentUser?.user_id]);

  // Rebuilds the connection if it dies while the app is open. Every room rides
  // one connection, so watching the active one covers all of them.
  // Twitch only: Kick rooms ride their own adapter, whose health the connection
  // store watches itself.
  useChatWatchdog(isTwitchRoom ? activeChannel : null);

  const isModerator = isTwitchRoom && isModeratorFrom(userBadges);
  /** Every Helix mod action keys off the channel's numeric id. */
  const broadcasterId = activeTab?.channelId ?? null;
  // The room's chat modes in Rust's words (services/room_modes.rs), shown in
  // the composer while the setting is on.
  const showRoomModes = useAppStore((s) => s.settings.chat_input?.show_room_modes ?? true);
  const roomModesAsPrompt = useAppStore((s) => s.settings.chat_input?.room_modes_style !== 'chip');
  const roomModes =
    showRoomModes && roomState?.modes && roomState.modes.modes.length > 0 ? roomState.modes : null;
  // Whether you can send here is Rust's call (your badges and your own follow
  // of a followers-only room); the chat menu lists the modes in the same words.
  const gating = useMemo<ChatGating>(() => {
    const view = roomState?.modes;
    return {
      labels: view ? view.modes.map((m) => m.label) : [],
      blocked: view?.blocked ?? null,
      reason: view?.reason ?? null,
      view: view && view.modes.length > 0 ? view : null,
      warning: view?.warning ?? null,
    };
  }, [roomState]);
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
          // Namespaced the way the desktop keys this store: bare for Twitch,
          // `provider:id` otherwise. Only Twitch carries a `user-id` tag, so a
          // Kick or YouTube chatter falls through to the raw platform id, and a
          // bare Kick id is indistinguishable from a Twitch one — same numeric
          // range, so the two chatters would share a row, share cosmetics, and
          // overwrite each other in the mention index.
          const rawId = backend.tags['user-id'] || backend.user_id;
          const msgProvider = backend.provider ?? 'twitch';
          userId = rawId
            ? msgProvider === 'twitch'
              ? rawId
              : `${msgProvider}:${rawId}`
            : undefined;
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

  // Scrolling back to the bottom resumes, the way tapping Resume does.
  //
  // This used to gate on the list's `isUserScroll`, which the list defines as
  // "up-intent AND more than 50px from the bottom". Resume needs LESS than
  // 30px. The two never overlap, so a scroll back down could never resume and
  // the Resume button was the only way back to live. The guard existed for
  // the read-back oscillation (a 6px drag pausing, then the same drag reading
  // as "at the bottom" and un-pausing under the finger). The honest test for
  // that is distance, not intent: resume only once the reader has actually
  // been AWAY from the bottom and then come back to it. A read-back that never
  // leaves the bottom band stays paused; a real return, dragged or flung,
  // lands live. ChatMessageList already withholds its own programmatic scrolls
  // from this callback, so nothing here fires for an auto-scroll.
  const wasAwayRef = useRef(false);
  const onScroll = useCallback(
    (distanceToBottom: number) => {
      if (!isPausedRef.current) {
        wasAwayRef.current = false;
        return;
      }
      if (distanceToBottom > 80) {
        wasAwayRef.current = true;
        return;
      }
      if (distanceToBottom < 30 && wasAwayRef.current) {
        wasAwayRef.current = false;
        pause(false);
      }
    },
    [pause],
  );

  // Returning to live: unpause, then let the list glide smoothly to the bottom
  // (ChatMessageList fills this ref with its eased scroll for exactly this).
  const listScrollToBottomRef = useRef<(() => void) | null>(null);
  const resumeLive = useCallback(() => {
    // `force`: this is an explicit tap on the Resume control, so it must never
    // be swallowed by the settle window the way a stray scroll report should be.
    pause(false, { force: true });
    // Same tick as the unpause: a frame later the re-render has already pinned
    // the list to the bottom and the glide finds nothing left to cover.
    listScrollToBottomRef.current?.();
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
      // The flash marks the message for ~2 s after the glide lands: long
      // enough to find it by eye, short enough not to read as a selection. The
      // same flash a mention jump uses, and it shows on a parent that is
      // itself a mention, which the row's own highlight prop could not.
      const ok = scrollChatToMessage(parentMsgId, { align: 'center', flash: true });
      if (!ok) addToast('Original message is no longer in chat history', 'info');
    },
    [pause, addToast],
  );

  // Opening a mention (an inbox row or a notification) lands here once its
  // chat is on screen: scroll and flash, or one backfill, or the context card.
  useMentionJump({
    activeChannel,
    messages,
    getMessageId,
    pause,
    login: activeTab?.login ?? null,
    provider: activeProvider,
    channelId: broadcasterId,
  });

  const onUsernameClick = useCallback(
    (
      userId: string,
      username: string,
      displayName: string,
      color: string,
      badges: Array<{ key: string; info: Record<string, unknown> }>,
    ) => {
      // The profile sheet is Helix-backed; a Kick chatter's id would open an
      // unrelated Twitch account.
      if (!isTwitchRoom) return;
      // The row's badges ride along: a worn sub badge is the live answer to
      // "subscribed here right now".
      setSheetUser({ userId, username, displayName, color, badges });
    },
    [isTwitchRoom],
  );

  /** Pull the author and body out of a rendered row's message id. */
  const describeMessage = useCallback(
    (messageId: string) => {
      // A held message has its own Allow and Deny, and nothing to reply to yet.
      if (isHeldRowId(messageId)) return null;
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

  // Slash command page actions (utils/slashUi.ts). /usercard and /user open the
  // profile sheet; /clearmessages hides what this room shows right now (visual
  // only: new rows still arrive, and another room is untouched).
  const [hiddenRows, setHiddenRows] = useState<{ channel: string; ids: ReadonlySet<string> } | null>(null);
  useEffect(() => {
    const openCard = (e: Event) => {
      const d = (e as CustomEvent<{ userId?: string; username?: string; displayName?: string }>).detail;
      if (!isTwitchRoom || !d?.userId || !d.username) return;
      setSheetUser({ userId: d.userId, username: d.username, displayName: d.displayName ?? d.username, color: '' });
    };
    const clearRows = () => {
      if (!activeChannel) return;
      const ids = new Set<string>();
      for (const message of messages) {
        const id = getMessageId(message);
        if (id) ids.add(id);
      }
      setHiddenRows({ channel: activeChannel, ids });
    };
    window.addEventListener('streamnook-open-user-card', openCard);
    window.addEventListener('streamnook-clear-local-chat', clearRows);
    return () => {
      window.removeEventListener('streamnook-open-user-card', openCard);
      window.removeEventListener('streamnook-clear-local-chat', clearRows);
    };
  }, [isTwitchRoom, activeChannel, messages, getMessageId]);
  // Messages AutoMod held, inline where they arrived, for a moderator of this
  // room: Allow and Deny on the row, which folds once someone decides it.
  const heldKey = isModerator && activeChannel ? activeChannel.toLowerCase() : '';
  const heldNow = useAutomodStore((s) => (heldKey ? s.held[heldKey] : undefined));
  const decidedNow = useAutomodStore((s) => (heldKey ? s.decided[heldKey] : undefined));
  useEffect(() => {
    if (heldKey) void useAutomodStore.getState().seed(heldKey);
  }, [heldKey]);
  const heldMessages = useMemo(() => heldRows(heldNow, decidedNow).map(heldRowMessage), [heldNow, decidedNow]);
  const shownMessages = useMemo(() => {
    const visible =
      !hiddenRows || hiddenRows.channel !== activeChannel
        ? messages
        : messages.filter((m) => {
            const id = getMessageId(m);
            return !id || !hiddenRows.ids.has(id);
          });
    return mergeHeld(visible, heldMessages);
  }, [hiddenRows, activeChannel, messages, getMessageId, heldMessages]);

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

  // Swiping a row to moderate it is on where moderator tools are, never in
  // the read-only overlay.
  const swipeSetting = useAppStore((s) => s.settings.chat_design?.mod_slider ?? true);
  const holdModButtons = useAppStore((s) => s.settings.chat_design?.mod_hold_buttons ?? true);
  const swipeOn = swipeSetting && modToolsArmed && !!broadcasterId && !noSend;
  const press = useLongPressDrag({
    resolve: resolveRow,
    onArm: onArmFan,
    scrollLockRef: listRef,
    // A sideways drift gives the hold up sooner, so the fan never opens over a swipe.
    cancelSlop: swipeOn ? 6 : undefined,
  });

  const closeFan = useCallback(() => {
    setFanTarget(null);
    press.release();
  }, [press]);

  // Swipe right on a message to moderate it. Rust lays the slider out for each
  // press, from where the thumb landed and with the chatter's name in its
  // words; the saved steps, Length and Style are read there.
  const sliderFor = useCallback(
    (rowWidth: number, startX: number, target: SwipeTarget) =>
      invoke<TouchSlider>('get_touch_mod_slider', {
        provider: 'twitch',
        rowWidth,
        startX,
        targetName: target.name,
        targetId: target.userId,
        targetBadges: target.badges,
        channel: activeTab?.login ?? activeChannel,
      }).catch((err) => {
        Logger.debug('[Swipe] slider unavailable:', err);
        return null;
      }),
    [activeTab?.login, activeChannel],
  );
  // The fan's quick Timeout is the swipe's first timeout length.
  const sliderSettings = useAppStore((s) => (s.settings.chat_design?.mod_slider_timeouts ?? []).join(','));
  const [quickSeconds, setQuickSeconds] = useState<number | undefined>(undefined);
  useEffect(() => {
    if (!swipeOn) return;
    let cancelled = false;
    void invoke<TouchSlider>('get_touch_mod_slider', { provider: 'twitch', rowWidth: 390 })
      .then((slider) => !cancelled && setQuickSeconds(slider.quick_seconds))
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [swipeOn, sliderSettings]);
  const [hintLearned, setHintLearned] = useState(() => {
    try {
      return localStorage.getItem(SWIPE_HINT_KEY) === '1';
    } catch {
      return false;
    }
  });
  const learnSwipe = useCallback(() => {
    setHintLearned(true);
    try {
      localStorage.setItem(SWIPE_HINT_KEY, '1');
    } catch {
      // Private storage is off; the hint just shows again next time.
    }
  }, []);

  // The viewer's standing here, for what Twitch allows on each message.
  const viewerRole: 'broadcaster' | 'moderator' | null = !modToolsArmed
    ? null
    : !!broadcasterId && currentUser?.user_id === broadcasterId
      ? 'broadcaster'
      : 'moderator';

  /** The chatter behind a row and what Twitch allows on it (`messageReach`):
   *  Delete alone on the viewer's own message, nothing on the broadcaster's. */
  const reachOf = useCallback(
    (messageId: string): { target: SwipeTarget; reach: MessageReach } | null => {
      if (isHeldRowId(messageId)) return null;
      for (const message of messages) {
        if (getMessageId(message) !== messageId) continue;
        try {
          const parsed = parseMessage(message as string | BackendChatMessage);
          const userId = parsed.tags.get('user-id') ?? '';
          if (!userId || parsed.username === 'System') return null;
          // Subs, raids and other event rows are the room's, not a chatter's line.
          if (parsed.tags.get('system-msg')) return null;
          const badges = (parsed.tags.get('badges') ?? '').split(',').filter(Boolean);
          const target = { userId, name: parsed.tags.get('display-name') || parsed.username || '', badges };
          return { target, reach: messageReach(viewerRole, currentUser?.user_id ?? '', userId, badges) };
        } catch {
          return null;
        }
      }
      return null;
    },
    [messages, getMessageId, viewerRole, currentUser?.user_id],
  );
  const swipeTargetFor = useCallback(
    (messageId: string): SwipeTarget | null => {
      const found = reachOf(messageId);
      if (found && found.reach !== 'nothing') return found.target;
      // Why a press starts no swipe, for the app log (the iPhone logs the same).
      const why = !found
        ? 'no chatter on this row'
        : `Twitch allows nothing on it (own ${found.target.userId === currentUser?.user_id}, role ${viewerRole ?? 'viewer'}, sender badges ${found.target.badges.join(',') || 'none'})`;
      Logger.warn('[Swipe]', `down on ${messageId}: not swipeable: ${why}`);
      return null;
    },
    [reachOf, viewerRole, currentUser?.user_id],
  );
  // The room's resolved role, logged when it changes.
  useEffect(() => {
    if (!isTwitchRoom) return;
    Logger.warn(
      '[Swipe]',
      `room ${activeTab?.login ?? activeChannel}: role ${viewerRole ?? 'viewer'}, viewer ${currentUser?.user_id || 'unknown'}, mod tools ${modToolsArmed ? 'on' : 'off'}, room id ${broadcasterId ?? 'unknown'}`,
    );
  }, [isTwitchRoom, viewerRole, modToolsArmed, broadcasterId, currentUser?.user_id, activeTab?.login, activeChannel]);

  // What the fan leaves out: by platform and the read-only overlay, and the mod
  // tiles while Hold menu buttons is off.
  const fanHides = useMemo(() => {
    const base = isTwitchRoom
      ? noSend
        ? READ_ONLY_HIDES
        : undefined
      : noSend
        ? OTHER_PLATFORM_READ_ONLY_HIDES
        : OTHER_PLATFORM_HIDES;
    if (!holdModButtons) return [...(base ?? []), ...MOD_TILES];
    // Only what Twitch allows on the held message: Delete alone on your own.
    const reach = fanTarget ? (reachOf(fanTarget.messageId)?.reach ?? 'nothing') : 'all';
    const refused: FanAction[] = [
      ...(reachCanDelete(reach) ? [] : (['delete'] as FanAction[])),
      ...(reachCanTimeout(reach) ? [] : (['timeout', 'ban'] as FanAction[])),
    ];
    return refused.length ? [...(base ?? []), ...refused] : base;
  }, [isTwitchRoom, noSend, holdModButtons, fanTarget, reachOf]);

  // Out of sight, nothing of this room stays up: the fan and the sheets draw
  // outside the pane, so hiding the pane would not hide them.
  useEffect(() => {
    if (!paused) return;
    closeFan();
    setSheetUser(null);
    setAddChatOpen(false);
  }, [paused, closeFan]);

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
        // The same door as desktop's hover dock: Rust's words and Undo on
        // success, Twitch's reason on a refusal, and the first saved reason on
        // a timeout or ban.
        case 'delete':
          if (broadcasterId) {
            void moderateWithToast({
              provider: 'twitch',
              broadcasterId,
              targetUserId: t.userId,
              targetName: t.username,
              action: { kind: 'delete', messageId: t.messageId },
            });
          }
          break;
        case 'timeout':
          if (broadcasterId && t.userId) {
            void moderateWithToast({
              provider: 'twitch',
              broadcasterId,
              targetUserId: t.userId,
              targetName: t.username,
              action: { kind: 'timeout', seconds: timeoutSecs ?? 600 },
              useSavedReason: true,
            });
          }
          break;
        case 'ban':
          if (broadcasterId && t.userId) {
            void moderateWithToast({
              provider: 'twitch',
              broadcasterId,
              targetUserId: t.userId,
              targetName: t.username,
              action: { kind: 'ban' },
              useSavedReason: true,
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
      if (noSend || !isPaused || press.isArmed()) return;
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
    [noSend, isPaused, press, describeMessage, activeChannel],
  );


  // The inbox over this chat: its bottom sheet, or a new mention's card. The
  // watch pane stays mounted under the mini player and behind Chats, so the
  // card rises only where the chat is on screen.
  const navTab = useMobileNavStore((s) => s.activeTab);
  const playerFull = useMobileNavStore((s) => s.playerMode === 'full');
  const onScreen = !paused && !idle && (surface === 'chats' || (navTab !== 'chats' && playerFull));
  const inboxSide = useAppStore((s) => (s.settings.chat_input?.inbox_button_side === 'right' ? 'right' : 'left'));
  const cardHere = useMentionsStore((s) => !!s.arrival && !s.composerBusy && !s.sheetIn) && onScreen && !readOnly;
  // Rows under the card fade out above it instead of showing as slivers; the
  // top fade stays when the pane draws one.
  const clear = cardHere ? '62px' : null;
  const listMask = clear
    ? `linear-gradient(to bottom, ${strip ? '#000 0' : 'transparent 0, #000 24px'}, #000 calc(100% - ${clear} - 24px), transparent calc(100% - ${clear}))`
    : undefined;

  return (
    <div className="flex-1 min-h-0 flex flex-col">
      {strip && <ChatTabStrip />}
      {top}
      <div className="flex-1 min-h-0 relative flex flex-col">
      <div
        ref={listRef}
        style={listMask ? { WebkitMaskImage: listMask, maskImage: listMask } : undefined}
        data-chat-list=""
        // `select-none` is load-bearing, not tidiness: Android WebView answers a
        // long-press on text by starting its OWN text selection, which wins the
        // gesture and the fan never arms. Chat text needs no selection anyway —
        // the fan's Copy bucket is the way to take a message.
        // With swiping on, a sideways move on a row is the swipe's from the first pixel: `sn-swipe-mod`
        // gives the rows themselves `touch-action: pan-y`. On this wrapper it would do nothing, since
        // the browser reads touch-action only from the touched element up to its scroll container.
        className={`flex-1 min-h-0 relative select-none [-webkit-touch-callout:none] ${strip || listMask ? '' : 'sn-fade-top'} ${swipeOn ? 'sn-swipe-mod' : ''}`}
        onContextMenu={onListContextMenu}
        onPointerDown={press.onPointerDown}
        onPointerMove={press.onPointerMove}
        onPointerUp={press.onPointerUp}
        onPointerCancel={press.onPointerCancel}
        onClick={onListClick}
        // While paused, rows are tappable to reply. Signposted by the hint pill
        // below rather than left as a secret gesture.
        data-tap-reply={isPaused && !noSend ? 'true' : undefined}
      >
        <HoldWhile hold={idle}>
          <ChatMessageList
            scrollToBottomRef={listScrollToBottomRef}
            messages={shownMessages}
            emptyNote={replayOn ? replayError : historyLoaded ? 'No recent messages' : null}
            renderToken={renderToken}
            isPaused={isPaused}
            onPauseIntent={() => pause(true)}
            onScroll={onScroll}
            onUsernameClick={onUsernameClick}
            onReplyClick={handleReplyClick}
            onEmoteRightClick={() => {}}
            onUsernameRightClick={() => {}}
            onBadgeClick={() => {}}
            highlightedMessageId={null}
            deletedMessageIds={deletedMessageIds}
            clearedUserContexts={clearedUserContexts}
            emotes={emotes}
            // The room's own platform: rows from it carry no platform mark, which
            // is only for telling sources apart in a combined chat.
            homeProvider={activeProvider}
            getMessageId={getMessageId}
          />
        </HoldWhile>
        {swipeOn && !hintLearned && (
          <div className="pointer-events-none absolute left-0 right-0 top-2 z-10 flex justify-center">
            {/* Only the X takes a touch: a swipe on the row under the pill still starts. */}
            <div className="glass-badge flex items-center gap-1.5 rounded-full py-1 pl-3 pr-1.5 text-[12px] text-textPrimary">
              <HandPointing size={14} weight="bold" className="text-accent" />
              Swipe right on a message to moderate it
              <button type="button" onClick={learnSwipe} aria-label="Dismiss the swipe hint" className="pointer-events-auto p-1 text-textMuted">
                <X size={11} weight="bold" />
              </button>
            </div>
          </div>
        )}
        {isPaused && (
          <div className="absolute bottom-2 left-0 right-0 flex flex-col items-center gap-1 z-10 pointer-events-none">
            {!noSend && (
              <span className="glass-badge rounded-full px-2 py-0.5 text-[10.5px] text-textMuted">
                Tap a message to reply
              </span>
            )}
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
      {/* Outside the list, so a press in them never reaches its gestures. */}
      {onScreen && !readOnly && <InboxArrival side={inboxSide} />}
      <InboxSheet surface={surface} />
      </div>
      {/* The landscape overlay has no room for it and shows whichever is chosen. */}
      {vodHere && !readOnly && (
        <div
          className="px-2.5 pt-1.5"
          // With no composer under it, the switch clears the gesture bar itself.
          style={replayOn ? { paddingBottom: 'calc(var(--sn-safe-b, 0px) + 10px)' } : undefined}
        >
          <SegmentedControl
            label="Chat for this video"
            segments={REPLAY_SEGMENTS}
            value={replayShowing}
            onChange={setVodReplayShowing}
            fill
          />
        </div>
      )}
      {!noSend && (
        <MobileChatInput
          inboxSurface={surface}
          channel={activeTab?.login ?? null}
          provider={activeProvider}
          channelId={broadcasterId}
          channelLabel={activeTab?.label ?? null}
          gating={gating}
          roomModes={roomModes}
          roomModesAsPrompt={roomModesAsPrompt}
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
      )}
      {/* Channel context matters: Twitch badges like sub tiers and moderator are
          scoped to the room, so without it the profile shows only global ones. */}
      <UserProfileSheet
        user={isTwitchRoom ? sheetUser : null}
        channelId={broadcasterId}
        channelName={activeChannel}
        onClose={handleCloseSheet}
      />
      <AddChatSheet open={addChatOpen} onClose={handleCloseAddChat} />
      {/* Long-press fan-out. Owns every per-message action now, for everyone:
          moderators simply get more buckets. */}
      <SwipeModerator
        listRef={listRef}
        enabled={swipeOn}
        broadcasterId={broadcasterId}
        targetFor={swipeTargetFor}
        sliderFor={sliderFor}
        fanArmed={press.isArmed}
        onDone={learnSwipe}
      />
      <ChatFanOut
        target={fanTarget}
        isModerator={modToolsArmed}
        canPin={modToolsArmed && !!broadcasterId}
        hide={fanHides}
        onCommit={runFanAction}
        quickSeconds={quickSeconds}
        onCancel={closeFan}
      />
    </div>
  );
};
