// Lean touch chat composer: text send, the shared emote/emoji picker, the
// channel wallet (points and bits), and a menu that shares the trailing slot
// with Send. Commands (slash and your own plain-text ones) run in Rust like the
// desktop's; resub/streak modes, send-as switching and arrow history stay
// desktop affordances.
//
// The picker and the wallet's tabs are the SAME components the desktop composer
// uses. Both are prop-driven, carry no window/hover dependencies, and anchor
// themselves with `absolute bottom-full` — so they only need a `relative`
// ancestor here, and mobile gets provider tabs, favorites, emoji and search for
// free instead of a second, worse implementation.
//
// Channel comes in as a prop rather than being read off `currentStream`: with
// several chat tabs open the composer must target the tab you are looking at,
// which is not necessarily the stream playing.
import React, { Fragment, useCallback, useEffect, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { DotsThreeVertical, WarningCircle, X } from 'phosphor-react';
import { emotePanelHeight, useLastKeyboardHeight } from '../ui/keyboardHeight';
import { CircleSlash, Inbox as InboxIcon, X as XIcon } from 'lucide-react';
import { KeyboardHideIcon, KeyboardShowIcon } from '../ui/KeyboardHideIcon';
import { useAppStore } from '../../stores/AppStore';
import { refreshChannelEmotes, sendChannelMessage, useChatConnectionStore } from '../../stores/chatConnectionStore';
import { sliceLookupKey } from '../../utils/providerKey';
import { varyRepeat } from '../../utils/sentContent';
import type { EmoteSet } from '../../services/emoteService';
import { Logger } from '../../utils/logger';
import { EmotePickerPanel, useSwappingSmiley } from '../../components/chat/EmotePickerPanel';
import { CheerBar } from '../../components/chat/CheerBar';
import { useCheerDraft } from '../../stores/cheerPromptStore';
import { WalletSheet } from '../../components/chat/ChannelWallet';
import { SplitWalletGlyph, WalletChestGlyph } from '../../components/chat/SplitWalletGlyph';
import { ChannelPointsIcon } from '../../components/ChannelPointsIcon';
import { useChannelPoints } from './useChannelPoints';
import { MobileEmoteCarousel } from './MobileEmoteCarousel';
import { getWordRange, type EmoteTabCandidate } from '../../utils/chatInputWord';
import { matchEmoteFill, rowsToTabCandidates, createRequestSeq } from '../../services/emoteMatch';
import { ComposerMenuSheet } from './ComposerMenuSheet';
import { closeInboxSheet, setComposerBusy, toggleInboxSheet, useMentionsStore, type InboxSurface } from '../mentions/mentionInbox';
import { fitCount } from '../../utils/fitNumber';
import { InboxTray } from '../mentions/InboxTray';
import { INBOX_SPRING } from '../mentions/inboxText';
import { MobileSheet } from '../ui/MobileSheet';
import { MobileCommandSheet } from './MobileCommandSheet';
import { applySlashOutcome, handleSlashCommand, runSlashCommand, type SlashOutcome } from '../../utils/commandHandler';
import { StreakBanners, type ArmedShare } from './StreakBanners';
import type { ChatWarningView, RoomModeKind, RoomModeView, SendChannelOutcome } from '../../stores/chatConnectionStore';
import { WarningBox } from './WarningBox';
import { RoomModeIndicator } from '../../components/chat/RoomModeIndicator';
import type { ProviderId } from '../../types/providers';
import { isTwitchStream } from '../../utils/streamProvider';
import { formatShortCount } from '../../utils/streamStats';
import { usePlatformAccountStore } from '../../stores/platformAccountStore';
import { ManageChannelSheet } from './ManageChannelSheet';
import { ModerationSheet } from './ModerationSheet';
import { useHiddenPinWaiting, usePinsOpen } from '../watch/pinsOpen';
import { useOutsidePress } from '../../hooks/useOutsidePress';

// The emote menu takes the keyboard's place, at the keyboard's last height,
// the platform's own emoji-keyboard pattern: the emote button swaps one for
// the other and the composer stays where it was, so the field can be watched
// filling while several emotes are tapped. It fills the wrapper that sizes it.
const PANEL_CLASS = 'h-full w-full border-t border-borderSubtle flex flex-col overflow-hidden !bg-background';

/** About 12 characters of the placeholder at 15 px: the least the field
 *  keeps before the inbox tray leaves a narrow bar. */
const FIELD_MIN_PX = 92;
const FOLD_HYSTERESIS_PX = 12;
/** What the tray takes from the field: its 40 px and the 6 px gap. */
const TRAY_ROOM = 46;

/** One of the right end's marks, stacked in the same spot and crossfaded. */
const END_MARK = 'absolute inset-0 flex items-center justify-center transition-opacity duration-200 pointer-events-none';

/** Twitch's message limit; the counter shows from COUNTER_FROM characters. */
const MESSAGE_LIMIT = 500;
const COUNTER_FROM = 400;

/** A message the room did not take: the text as sent and why. */
interface SendFailure {
  text: string;
  why: string;
}

/** An invoke rejection in words: Rust commands reject with a plain string. */
function failureWords(err: unknown): string {
  if (typeof err === 'string' && err.trim()) return err;
  if (err instanceof Error && err.message) return err.message;
  return 'something went wrong';
}

/** Whether you can send in this room, as Rust decided it (room_modes.rs). */
export interface ChatGating {
  /** The room's active modes in words. */
  labels: string[];
  /** The mode that certainly keeps you from sending, or null. */
  blocked: RoomModeKind | null;
  /** What the disabled field says while blocked. */
  reason: string | null;
  /** The room's modes in full for the chat menu, whatever the composer's
   *  room-mode icons are set to. */
  view?: RoomModeView | null;
  /** A moderator's warning you have not acknowledged: the field stays locked
   *  behind the Acknowledge box above it. */
  warning?: ChatWarningView | null;
}

interface Props {
  /** The room's bare name on its platform (login or slug). */
  channel: string | null;
  /** The platform the room is on. Only Twitch rooms get points, commands and
   *  the Twitch send path; anything else sends through its own adapter. */
  provider: ProviderId;
  channelId: string | null;
  channelLabel: string | null;
  gating: ChatGating;
  /** The room's chat modes in Rust's words, or null while hidden or none. */
  roomModes?: RoomModeView | null;
  /** Name the modes in place of "Send a message" instead of the chip. */
  roomModesAsPrompt?: boolean;
  emotes: EmoteSet | null;
  replyTo?: { messageId: string; username: string } | null;
  onCancelReply?: () => void;
  isModerator: boolean;
  modToolsOn: boolean;
  onToggleModTools: () => void;
  onAddChat: () => void;
  onReload: () => void;
  /** Absent for the stream-following tab, which is not closable. */
  onCloseChat?: () => void;
  /** Which chat's inbox sheet the tray opens. */
  inboxSurface?: InboxSurface;
}

// Memoized below. The pane re-renders on every arriving chat message and the
// composer's props barely ever change, so without this the whole input subtree
// (and the emote work behind it) is rebuilt at chat's message rate. The memo is
// only worth anything while every callback prop stays referentially stable, so
// keep them in useCallback on the pane side.
const MobileChatInputImpl: React.FC<Props> = ({
  channel,
  provider,
  channelId,
  channelLabel,
  gating,
  roomModes = null,
  roomModesAsPrompt = false,
  emotes,
  replyTo,
  onCancelReply,
  isModerator,
  modToolsOn,
  onToggleModTools,
  onAddChat,
  onReload,
  onCloseChat,
  inboxSurface = 'watch',
}) => {
  const currentUser = useAppStore((s) => s.currentUser);
  const chatInput = useAppStore((s) => s.settings.chat_input);
  const isTwitch = provider === 'twitch';
  const kickConnected = usePlatformAccountStore((s) => s.kick.connected);
  const kickBusy = usePlatformAccountStore((s) => s.kick.busy);
  const needsKickSignIn = provider === 'kick' && !kickConnected;
  // One draft per room. A single field shared by every tab carried what you
  // typed in one room into the next, and Send then posted it there. A send
  // that is still in flight answers into the room it was typed in.
  const roomKey = channel ? `${provider}:${channel.toLowerCase()}` : '';
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [manageOpen, setManageOpen] = useState(false);
  const [moderationOpen, setModerationOpen] = useState(false);
  // A pin folded away by Pinned Messages > Style: Hidden opens from the menu.
  const pins = usePinsOpen();
  const hiddenPinWaiting = useHiddenPinWaiting();
  const text = drafts[roomKey] ?? '';
  const setDraftFor = useCallback((key: string, next: string | ((prev: string) => string)) => {
    setDrafts((d) => {
      const prev = d[key] ?? '';
      const value = typeof next === 'function' ? next(prev) : next;
      if (value === prev) return d;
      const out = { ...d };
      if (value) out[key] = value;
      else delete out[key];
      return out;
    });
  }, []);
  const setText = (next: string | ((prev: string) => string)) => setDraftFor(roomKey, next);
  // A message the room did not take stays in its draft with why, until it is
  // sent again or discarded. Editing it makes it a new message, so the line
  // shows only while the field still holds the text that failed.
  const [failures, setFailures] = useState<Record<string, SendFailure>>({});
  const setFailureFor = useCallback((key: string, failure: SendFailure | null) => {
    setFailures((f) => {
      if (!failure && !(key in f)) return f;
      const out = { ...f };
      if (failure) out[key] = failure;
      else delete out[key];
      return out;
    });
  }, []);
  const failure = failures[roomKey];
  const shownFailure = failure && text.trim() === failure.text ? failure : null;
  const [emotesOpen, setEmotesOpen] = useState(false);
  // The panel's height: the keyboard it stands in for.
  const keyboardHeight = useLastKeyboardHeight();
  // A swipe down on the panel's top (search and tabs) puts it away.
  const panelSwipe = useRef<number | null>(null);
  const emoteButtonRef = useRef<HTMLButtonElement>(null);
  // The whole composer: the emote picker's company (typing, sending and the
  // emote button leave it open, as on the iPhone) and the suggestions'.
  const composerRef = useRef<HTMLDivElement>(null);
  // The draft the suggestions were put away for by a tap outside; typing
  // brings them back.
  const [suggestionsHiddenFor, setSuggestionsHiddenFor] = useState<string | null>(null);
  const [pointsOpen, setPointsOpen] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  // The inbox tray's end of the bar (Settings > Chat > Inbox button); the chat
  // menu (and Send, which shares its slot) takes the other end.
  const trayRight = chatInput?.inbox_button_side === 'right';
  // A message bar dragged narrow (the side chat beside the video) gives the
  // field priority: the tray stays off the bar while it would leave the field
  // under about 12 characters, and the chat menu carries the inbox and its
  // count instead. Read from the field's real width, so any narrow bar
  // behaves the same; the gap between the two thresholds keeps it from
  // flickering at the edge.
  const [trayFolded, setTrayFolded] = useState(false);
  const inboxUnread = useMentionsStore((s) => s.unreadTotal);
  const [commandsOpen, setCommandsOpen] = useState(false);
  const [sending, setSending] = useState(false);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  // A streak share the banner armed: the next send goes to it instead of to
  // chat, with whatever is typed (nothing typed is fine).
  const armedShare = useRef<ArmedShare | null>(null);
  const [shareArmed, setShareArmed] = useState<'watch' | 'resub' | null>(null);
  // Whether the field has the keyboard: while it does, the commands button is
  // the way to put the keyboard away (scrolling the chat never does).
  const [typing, setTyping] = useState(false);
  // The composer rows of Settings > Chat that this field honors.
  const spellcheck = chatInput?.spellcheck_enabled ?? true;
  const hidePlaceholder = chatInput?.hide_placeholder ?? false;
  const hideEmoteButton = chatInput?.hide_emote_button ?? false;
  // The last line sent to each room, as it went out, for Duplicate sends.
  const lastSent = useRef<Map<string, string>>(new Map());

  // Slow mode: when each room lets you send again, counted from your last
  // message there. The wait is Rust's (`slow_seconds`), already 0 for anyone
  // slow mode spares, so a moderator never sees a countdown.
  const [slowUntil, setSlowUntil] = useState<Record<string, number>>({});
  const [clock, setClock] = useState(() => Date.now());
  const until = slowUntil[roomKey] ?? 0;
  const slowWait = until > clock ? Math.ceil((until - clock) / 1000) : 0;
  useEffect(() => {
    if (until <= 0) return;
    const id = window.setInterval(() => {
      const t = Date.now();
      setClock(t);
      if (t >= until) window.clearInterval(id);
    }, 250);
    return () => window.clearInterval(id);
  }, [until]);
  const startSlowWait = (key: string, ch: string) => {
    const slice = useChatConnectionStore.getState().channels.get(sliceLookupKey(provider, ch));
    const wait = slice?.roomState.modes?.slow_seconds ?? 0;
    const now = Date.now();
    setClock(now);
    setSlowUntil((s) => {
      if (wait <= 0 && !(key in s)) return s;
      const out = { ...s };
      if (wait > 0) out[key] = now + wait * 1000;
      else delete out[key];
      return out;
    });
  };
  const { currentSmiley, isSmileyTransitioning, cycleEmoteSmiley } = useSwappingSmiley();
  // The wallet follows the chat you are typing in, like everything else in
  // this composer: a cheer posts in that room, and its rewards are that
  // channel's. Before, the points readout followed the watched stream while
  // the rewards sheet listed the focused tab's channel, so with a second tab
  // focused one channel's rewards were judged against another's balance.
  //
  // The watched stream's bonus chest still has to be collected when you are
  // typing somewhere else: Twitch only offers a chest on the channel it
  // believes you watch, and on the phone nothing else collects it. A second
  // hook on the stream does that and shows nothing.
  //
  // Points are Twitch's. While a Kick stream plays, currentStream carries a Kick
  // numeric id, and handed to GQL that id names an unrelated Twitch channel.
  const streamIsTwitch = useAppStore((s) => isTwitchStream(s.currentStream));
  const streamLogin = useAppStore((s) => s.currentStream?.user_login);
  const streamId = useAppStore((s) => s.currentStream?.user_id);
  const points = useChannelPoints(isTwitch ? channel : null, isTwitch ? channelId : null);
  const streamElsewhere = streamIsTwitch && !!streamLogin && streamLogin.toLowerCase() !== (isTwitch ? channel?.toLowerCase() : null);
  useChannelPoints(streamElsewhere ? streamLogin : null, streamElsewhere ? streamId : null);
  const walletBits = points.bits;
  // The box's text cheers (Rust's live check): Send becomes the cheer.
  const composerCheer = useCheerDraft(isTwitch ? channel : null, text);
  const cheerBlocked = !!composerCheer && composerCheer.problems.length > 0;
  const hasWallet = isTwitch && !!channel && !!channelId && (points.balance !== null || !!walletBits);
  // Wallet balance: Always puts the short balance beside the icon; Hidden (and
  // a saved desktop On hover, which a touch cannot do) keeps just the icon.
  const balanceAlways = useAppStore(
    (s) => s.settings.chat_input?.show_points_balance_inline === true && s.settings.chat_input?.hide_points_balance !== true,
  );
  const shownBalances =
    balanceAlways && !pointsOpen && !points.availableClaimId
      ? [points.balance, walletBits?.balance ?? null].filter((n): n is number => n !== null)
      : [];
  // The channel's own points art, or Twitch's glyph.
  const pointsArt = (size: number) =>
    points.iconUrl ? (
      <img src={points.iconUrl} alt="" style={{ width: size, height: size }} className="object-contain" draggable={false} />
    ) : (
      <ChannelPointsIcon size={size} />
    );
  // One rewards read per channel while it has a points button, so the sheet's
  // first open paints Rust's list instead of waiting on Twitch.
  const hasPointsButton = !!channel && !!channelId && points.balance !== null;
  useEffect(() => {
    if (!hasPointsButton || !channel) return;
    invoke('get_channel_rewards', { channelId: channel }).catch(() => {});
  }, [hasPointsButton, channel]);

  // Points feedback: the hook turns each new gain Rust records (watching, the
  // chest, predictions, a rise only the poll saw) into one "+N".
  const flash = points.gain;
  const clearFlash = points.clearGain;

  const canSend = !!text.trim() || !!shareArmed;

  const send = async () => {
    if (armedShare.current) {
      const key = roomKey;
      const shared = await armedShare.current(text.trim());
      // Shared: the text went with it. Not: it stays for another try.
      if (shared) setDraftFor(key, '');
      return;
    }
    const message = text.trim();
    if (!message || !channel || sending || slowWait > 0) return;
    // Captured now: the answer lands in this room even if you switch tabs.
    const key = roomKey;
    const sent = () => {
      setDraftFor(key, '');
      setFailureFor(key, null);
    };
    const failed = (why: string) => setFailureFor(key, { text: message, why });
    if (!isTwitch) {
      // Slash commands are Twitch calls; typed here they would post as text.
      if (message.startsWith('/')) {
        useAppStore.getState().addToast('Commands only work in Twitch chats', 'info');
        return;
      }
      setSending(true);
      try {
        // The platform can accept the request and still refuse the message
        // (slow mode, followers-only, a banned word).
        const outcome = await invoke<{ is_sent: boolean; drop_reason: string | null }>(
          'provider_send_message',
          {
            provider,
            channel: channel.toLowerCase(),
            text: message,
            replyTo: replyTo?.messageId ?? null,
          },
        );
        if (outcome && outcome.is_sent === false) {
          failed(outcome.drop_reason || 'the room refused it');
          return;
        }
        sent();
        startSlowWait(key, channel);
        onCancelReply?.();
      } catch (err) {
        Logger.warn('[MobileChatInput] send failed:', err);
        failed(failureWords(err));
      } finally {
        setSending(false);
      }
      return;
    }
    if (!currentUser) {
      // Signed in with the profile still on its way (a session kept through a
      // network failure): keep the draft and say why, never drop it silently.
      if (useAppStore.getState().isAuthenticated) failed('your Twitch profile is still loading');
      return;
    }
    const identity = {
      username: currentUser.login || currentUser.username,
      displayName: currentUser.display_name || currentUser.username,
      userId: currentUser.user_id,
      color: undefined,
      badges: '',
    };
    // Chat text to the room. With Duplicate sends on, a repeat of the last
    // line sent here goes out varied or plain, whichever differs from that
    // line (varyRepeat, the desktop composer's rule); commands are never
    // deduplicated by Twitch, so they go as typed.
    const chatSend = async (msg: string, reply?: string) => {
      const text =
        !!useAppStore.getState().settings.chat_input?.bypass_duplicate && !msg.startsWith('/')
          ? varyRepeat(msg, lastSent.current.get(key))
          : msg;
      // What actually went out, so the next repeat flips the other way.
      lastSent.current.set(key, text.trimEnd());
      return sendChannelMessage(channel, text, identity, reply);
    };
    setSending(true);
    try {
      let outcome: SendChannelOutcome | null;
      // Slash commands are Helix/GQL calls on the desktop handler, never chat
      // text. The phone dropped them as "desktop affordances"; moderating from
      // a phone is not one. Same handler, same identity, the room's own
      // broadcaster id (a secondary tab's, not the stream's).
      if (message.startsWith('/')) {
        if (!channelId) {
          useAppStore.getState().addToast("Open this channel's stream to run commands here", 'info');
          return;
        }
        // A command may answer with text to send (/me); that send is judged
        // like any other, so a dropped one keeps the draft too.
        const chat: { pending?: Promise<SendChannelOutcome | null> } = {};
        const handled = await handleSlashCommand(message, channelId, channel, (msg) => {
          chat.pending = chatSend(msg);
        });
        if (handled && !chat.pending) {
          sent();
          return;
        }
        outcome = chat.pending ? await chat.pending : await chatSend(message, replyTo?.messageId);
      } else {
        // Plain text can be one of your own commands (those that need no
        // slash), which Rust runs like the slash ones. Only asked when one is
        // saved, so ordinary chat makes no extra call (the desktop's rule).
        // A dispatch that fails sends the text as typed: it is chat first.
        const own = useAppStore.getState().settings.chat_commands?.user_commands;
        let answer: SlashOutcome | null = null;
        if (channelId && own?.some((c) => c.enabled && c.require_slash === false)) {
          answer = await runSlashCommand(message, channelId, channel).catch((err: unknown) => {
            Logger.warn('[MobileChatInput] run_slash_command failed:', err);
            return null;
          });
        }
        if (answer?.handled) {
          const chat: { pending?: Promise<SendChannelOutcome | null> } = {};
          await applySlashOutcome(answer, channel, channelId ?? '', (msg) => {
            chat.pending = chatSend(msg);
          });
          if (!chat.pending) {
            sent();
            return;
          }
          outcome = await chat.pending;
        } else {
          outcome = await chatSend(message, replyTo?.messageId);
        }
      }
      if (!outcome) {
        failed('chat is not connected');
        return;
      }
      if (outcome.is_sent === false) {
        failed(outcome.drop_reason || 'the room refused it');
        return;
      }
      sent();
      startSlowWait(key, channel);
      onCancelReply?.();
    } catch (err) {
      Logger.error('[MobileChatInput] send failed:', err);
      failed(failureWords(err));
    } finally {
      setSending(false);
    }
  };

  const discardFailure = () => {
    setDraftFor(roomKey, '');
    setFailureFor(roomKey, null);
  };

  // The shared picker hands back the literal token to insert, which is an emote
  // name for emotes and the character itself for emoji.
  const insertEmote = (token: string) => {
    setText((t) => (t.length === 0 || t.endsWith(' ') ? `${t}${token} ` : `${t} ${token} `));
    if (!emotesOpen) inputRef.current?.focus();
  };

  // Suggestions for the word being typed.
  //
  // The emote menu is a fine way to browse, and a poor way to reach one emote
  // you already know the name of: it covers most of a phone screen to insert a
  // few characters. Typing the first few letters and tapping is the fast path,
  // and it is the one the desktop has had all along behind a key this platform
  // does not have.
  //
  // Completes the LAST word, rather than reading the caret. On a phone the
  // caret is essentially always at the end, and the real position would not
  // re-run this anyway: moving it changes no state. A trailing space falls out
  // of this correctly, since the word at the end is then empty.
  const [wordStart, wordEnd] = getWordRange(text, text.length);
  const word = text.slice(wordStart, wordEnd);
  // The desktop's emote fill menu, asked the same way (ranked in Rust over the
  // sets it already holds), so a word lists the same emotes in the same order
  // on every app. Each answer is kept with the word it answers, so a late
  // reply for an older word is never shown.
  const [answered, setAnswered] = useState<{ word: string; items: EmoteTabCandidate[] }>({ word: '', items: [] });
  const [suggestSeq] = useState(createRequestSeq);
  useEffect(() => {
    if (!word || !channel) return;
    const n = suggestSeq.next();
    // The answer carries the room's recent chatters after the emotes. One
    // letter is enough for the full list.
    void matchEmoteFill({ provider, channel, channelId }, word).then((res) => {
      if (!suggestSeq.isCurrent(n)) return;
      setAnswered({ word, items: rowsToTabCandidates(res.rows) });
    });
  }, [word, channel, channelId, provider, suggestSeq]);
  const suggestions = word && answered.word === word ? answered.items : [];

  // What the empty field says: why you cannot send when a mode stops you, the
  // slow-mode wait, else "Send a message" (nothing when Settings hides it). A
  // mode that only may hold you back is said once, by its icon at the field's
  // end; the chat menu names it in full.
  const placeholder = gating.warning
    ? gating.warning.prompt
    : shareArmed
    ? // The armed share rides the next send: the field says what it is for.
      `Add a message to your ${shareArmed === 'watch' ? 'watch streak' : 'resub'}`
    : gating.blocked
    ? (gating.reason ?? 'Chat restricted')
    : slowWait > 0
      ? `Slow mode: you can send again in ${slowWait}s`
      : hidePlaceholder
        ? ''
        : ((roomModesAsPrompt ? roomModes?.prompt : null) ?? 'Send a message');
  // Characters as Twitch counts them (code points, so an emoji is one).
  const charCount = text.length >= COUNTER_FROM ? Array.from(text).length : text.length;

  const showSuggestions =
    (chatInput?.emote_tab_complete_enabled ?? true) &&
    !emotesOpen &&
    !pointsOpen &&
    suggestions.length > 0 &&
    suggestionsHiddenFor !== text;
  // A tap outside the composer puts the suggestions away with the keyboard.
  const markSuggestionsInside = useOutsidePress(showSuggestions, [composerRef], () => setSuggestionsHiddenFor(text));

  // A new mention's card rises into the room just above the bar. The bar's own
  // risers use that room too (the suggestions strip over a draft, the emote
  // and wallet panels), so the card waits while one is up and shows once it
  // clears, after the send. A taller draft lifts the card with the bar.
  useEffect(() => {
    setComposerBusy(showSuggestions || emotesOpen || pointsOpen);
  }, [showSuggestions, emotesOpen, pointsOpen]);
  useEffect(() => () => setComposerBusy(false), []);

  useEffect(() => {
    const el = inputRef.current;
    if (!el) return;
    const read = () => {
      // Typing hides the tray anyway, so the bar's resting width is not on
      // screen to judge by; the call waits for the keyboard to go.
      if (document.activeElement === el) return;
      const w = el.clientWidth;
      setTrayFolded((folded) =>
        folded ? w - TRAY_ROOM < FIELD_MIN_PX + FOLD_HYSTERESIS_PX : w < FIELD_MIN_PX,
      );
    };
    read();
    const observer = new ResizeObserver(read);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);
  // What each end of the bar holds. Send is always the right end's job (see
  // the button); the swap setting decides which of the tray and the chat menu
  // sits left. A narrow bar folds the tray away and the menu takes the right.
  const leftEnd: 'tray' | 'menu' | null = trayFolded ? null : trayRight ? 'menu' : 'tray';
  const rightEnd: 'tray' | 'menu' = trayFolded ? 'menu' : trayRight ? 'tray' : 'menu';
  // While the emote panel holds the keyboard's place, the end brings the
  // keyboard back (Send still wins once there is a message).
  const endMode: 'send' | 'keyboardUp' | 'keyboard' | 'tray' | 'menu' = canSend
    ? 'send'
    : emotesOpen
      ? 'keyboardUp'
      : typing
        ? 'keyboard'
        : rightEnd;
  // Back to the keyboard: it comes up where the panel was.
  const backToKeyboard = () => {
    setEmotesOpen(false);
    inputRef.current?.focus();
  };
  const inboxOpenHere = useMentionsStore((s) => s.sheetIn === inboxSurface);
  const openMenu = () => {
    setEmotesOpen(false);
    setPointsOpen(false);
    setMenuOpen(true);
  };

  // Replace the word being typed rather than appending, which is what makes
  // this a completion instead of a second insertion.
  const completeWith = (token: string) => {
    setText((t) => {
      const [start] = getWordRange(t, t.length);
      return `${t.slice(0, start)}${token} `;
    });
    inputRef.current?.focus();
  };

  return (
    // Bottom padding: ride the soft keyboard (--sn-kb, pushed by the native
    // WindowInsets bridge; targetSdk 36 edge-to-edge ignores adjustResize) and
    // clear the gesture pill when the keyboard is closed. SUM, not max: the
    // bridge reports the keyboard height minus the gesture bar, so the full
    // lift above the keyboard is kb + safe-b (max left the input clipped by
    // exactly the gesture-bar height).
    <div
      ref={composerRef}
      data-chat-composer=""
      onPointerDownCapture={markSuggestionsInside}
      className="shrink-0 border-t border-borderSubtle"
      style={{
        paddingBottom: 'calc(var(--sn-kb, 0px) + var(--sn-safe-b, 0px) + 10px)',
        transition: 'padding-bottom 0.15s ease-out',
      }}
    >
      {/* Shareable streaks, same place desktop puts them: directly above the
          composer, because sharing sends whatever you have typed along with it. */}
      <StreakBanners
        channel={channel}
        channelId={channelId}
        armedRef={armedShare}
        onArmedChange={setShareArmed}
        focusComposer={() => inputRef.current?.focus()}
      />
      {/* No warning line here, deliberately. It rendered `gating.reason`, which
          is the exact string the disabled field's placeholder already shows, in
          the same warning colour -- the same sentence twice, stacked. The
          placeholder alone says why the field is dead, and it says it in the
          field you were about to type in. The chat menu carries the room's full
          state, and now the Follow action alongside it. */}
      {/* A moderator's warning: what they said, and Acknowledge, the one way to
          chat here again. Where the room's words live, so it cannot scroll away;
          Rust unlocks the field once Twitch takes the acknowledgement. */}
      {gating.warning && channelId && <WarningBox warning={gating.warning} roomId={channelId} />}
      {/* A message the room did not take. The text stays in the field; this
          says why once, beside it, with the two ways forward. */}
      {shownFailure && (
        <div className="flex items-center gap-1.5 px-3 pt-1.5 text-[12.5px]" role="alert">
          <WarningCircle size={14} weight="fill" className="shrink-0 text-warning" />
          <span className="min-w-0 flex-1 line-clamp-2 text-textSecondary">Not sent: {shownFailure.why}</span>
          <button
            onClick={() => void send()}
            disabled={sending || slowWait > 0}
            className="shrink-0 px-1.5 py-1 font-semibold text-accent active:opacity-70 disabled:opacity-50"
          >
            Retry
          </button>
          <button
            onClick={discardFailure}
            className="shrink-0 px-1.5 py-1 text-textMuted active:text-textPrimary"
          >
            Discard
          </button>
        </div>
      )}
      {replyTo && (
        <div className="flex items-center gap-1.5 px-3 pt-1.5 text-[12.5px] text-textSecondary">
          <span className="truncate">
            Replying to <span className="font-semibold">@{replyTo.username}</span>
          </span>
          <button
            onClick={onCancelReply}
            className="ml-auto p-1.5 text-textMuted active:text-textPrimary"
            aria-label="Cancel reply"
          >
            <X size={14} />
          </button>
        </div>
      )}
      {/* `relative` is the anchor both panels position against. */}
      <div className="relative flex items-end gap-1.5 px-2.5 pt-2">
        {/* Anchors the same way the panels do, and hides while either of them
            is open so two things never occupy that space at once. */}
        <AnimatePresence>
          {showSuggestions && (
            <MobileEmoteCarousel
              candidates={suggestions}
              onSelect={(tok) => completeWith(tok.insertText ?? tok.name)}
            />
          )}
        </AnimatePresence>
        {/* The bar's left end: the inbox tray, or the chat menu when Settings
            swaps the pair. While you type it slides out its own side and the
            field grows into its room, in one spring (width and transform
            only); it comes back the same way. A bar too narrow for the tray
            keeps nothing here: the chat menu takes the right end. */}
        {leftEnd && (
          <motion.div
            className="shrink-0 self-end"
            style={{ order: -1 }}
            initial={false}
            animate={{ width: typing ? 0 : 40, marginRight: typing ? -6 : 0 }}
            transition={INBOX_SPRING}
            aria-hidden={typing}
          >
            <motion.div
              initial={false}
              animate={{ x: typing ? -48 : 0, opacity: typing ? 0 : 1 }}
              transition={INBOX_SPRING}
              style={{ pointerEvents: typing ? 'none' : undefined }}
            >
              {leftEnd === 'tray' ? (
                <InboxTray surface={inboxSurface} side="left" />
              ) : (
                <button
                  onClick={openMenu}
                  className="chrome-glaze chrome-glaze--control relative shrink-0 flex items-center justify-center w-10 h-10 text-textSecondary active:text-textPrimary"
                  aria-label="Chat options"
                >
                  <DotsThreeVertical size={22} weight="bold" />
                </button>
              )}
            </motion.div>
          </motion.div>
        )}
        {/* The emote trigger and the points button live INSIDE the field so the
            typing area keeps full width instead of being squeezed by chrome.
            Exactly 40px, the height of the round button beside it. */}
        <div
          className={`glass-input flex-1 min-w-0 h-10 flex items-center gap-1 pr-2 ${hideEmoteButton ? 'pl-3' : 'pl-1'}`}
        >
          {!hideEmoteButton && (
            <button
              ref={emoteButtonRef}
              onClick={() => {
                setPointsOpen(false);
                if (emotesOpen) {
                  backToKeyboard();
                  return;
                }
                cycleEmoteSmiley();
                // The keyboard goes and the panel takes its place.
                inputRef.current?.blur();
                setEmotesOpen(true);
              }}
              className={`shrink-0 w-9 h-9 flex items-center justify-center text-[20px] leading-none active:opacity-70 ${
                emotesOpen ? 'text-accent' : ''
              }`}
              aria-label="Emotes and emoji"
              aria-pressed={emotesOpen}
            >
              {/* Always the emoji button; the way back to the keyboard is the
                  bar's right end while the panel is open. */}
              <span
                className="transition-[opacity,transform] duration-100"
                style={{
                  opacity: isSmileyTransitioning ? 0 : 1,
                  transform: isSmileyTransitioning ? 'scale(0.8)' : 'scale(1)',
                }}
              >
                {currentSmiley}
              </span>
            </button>
          )}
          {needsKickSignIn ? (
            <button
              onClick={() => void usePlatformAccountStore.getState().connect('kick')}
              disabled={kickBusy}
              className="flex-1 min-w-0 py-2 text-left text-[15px] font-medium text-accent disabled:opacity-60"
            >
              {kickBusy ? 'Waiting for you to sign in' : 'Sign in to Kick to chat'}
            </button>
          ) : (
            <textarea
              ref={inputRef}
              value={text}
              onChange={(e) => setText(e.target.value)}
              // Its own hide-keyboard button takes the menu's slot while typing.
              data-keyboard-hide="own"
              onFocus={() => {
                setTyping(true);
                setEmotesOpen(false);
                // The bar is for writing now: an open inbox goes first.
                closeInboxSheet();
              }}
              onBlur={() => setTyping(false)}
              onKeyDown={(e) => {
                // An IME's Enter confirms the composed word; it never sends.
                if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
                  e.preventDefault();
                  void send();
                }
              }}
              placeholder={placeholder}
              // Typing is switched off, not just discouraged, when the badges prove
              // the send would be rejected. The placeholder says why, in the
              // field's ordinary placeholder ink: a statement, not an alarm.
              disabled={!!gating.blocked || !!gating.warning}
              spellCheck={spellcheck}
              autoCorrect={spellcheck ? 'on' : 'off'}
              rows={1}
              className="flex-1 min-w-0 resize-none bg-transparent py-2 text-[15px] leading-[1.4] text-textPrimary outline-none max-h-[96px] placeholder:text-textMuted"
              enterKeyHint="send"
            />
          )}
          {/* Near Twitch's limit, the count takes the mode icon's place: quiet,
              orange in the last 20, red past it. Otherwise the room's chat
              modes, staying while you type; a tap opens the chat menu, which
              lists them in full. Hidden while a mode blocks you, since the
              placeholder already says that one. */}
          {!needsKickSignIn &&
            (charCount >= COUNTER_FROM ? (
              <span
                className={`shrink-0 h-9 flex items-center px-1 text-[12px] font-medium tabular-nums ${
                  charCount > MESSAGE_LIMIT
                    ? 'text-error'
                    : charCount >= MESSAGE_LIMIT - 20
                      ? 'text-warning'
                      : 'text-textMuted'
                }`}
                aria-label={`${charCount} of ${MESSAGE_LIMIT} characters`}
              >
                {charCount}/{MESSAGE_LIMIT}
              </span>
            ) : (
              roomModes &&
              !roomModesAsPrompt &&
              !gating.blocked && <RoomModeIndicator
                  view={roomModes}
                  inline
                  onActivate={() => {
                    setEmotesOpen(false);
                    setMenuOpen(true);
                  }}
                />
            ))}
          {/* The command list one tap from the field instead of two menus deep.
              Commands are Twitch calls (any other room would post the "/" as
              text), so only a Twitch room shows it. */}
          {isTwitch && channel && (
            <button
              onClick={() => {
                setEmotesOpen(false);
                setPointsOpen(false);
                setCommandsOpen(true);
              }}
              className="shrink-0 w-9 h-9 flex items-center justify-center text-textSecondary active:opacity-70"
              aria-label="Chat commands"
            >
              <CircleSlash size={19} strokeWidth={2} />
            </button>
          )}
          {/* Icon only unless Wallet balance is Always: a number at rest costs
              the message field real width, and the menu always prints it in
              full. What matters is the MOMENT of earning, which the flash and
              the floating amount carry. */}
          {hasWallet && (
            <button
              onClick={() => {
                // A waiting chest is collected on the way in. The rewards menu
                // has no concept of the chest, so without this the only route to
                // it would be turning auto-collect back on. Opening anyway is
                // deliberate: the menu is where the new balance shows up.
                if (points.availableClaimId) points.claimChest();
                setEmotesOpen(false);
                setPointsOpen((v) => !v);
              }}
              className={`relative shrink-0 h-9 flex items-center justify-center gap-1 active:opacity-70 ${
                shownBalances.length ? 'px-1.5' : 'w-9'
              } ${pointsOpen ? 'text-accent' : 'text-textSecondary'}`}
              aria-label={pointsOpen ? 'Close the wallet' : 'Wallet'}
            >
              {/* The amount, drifting up off the icon and dissolving.
                  Amber rather than the theme accent, on purpose: the accent is
                  the colour of everything else in the composer, so a credit
                  painted in it blended into the furniture. Amber is the one
                  thing on this bar wearing it, which is what makes a glance
                  catch it.

                  Keyed on a counter so two credits close together each get
                  their own animation instead of the second being swallowed as
                  "the same element". Centred via framer's own `x`, not a
                  Tailwind translate, since framer owns the transform property
                  and would overwrite it. */}
              <AnimatePresence>
                {flash && (
                  <motion.span
                    key={flash.id}
                    className="absolute left-1/2 pointer-events-none text-[15px] font-bold whitespace-nowrap"
                    style={{ color: 'var(--color-warning)' }}
                    initial={{ opacity: 0, x: '-50%', y: 0, scale: 0.65, filter: 'blur(0px)' }}
                    animate={{
                      opacity: [0, 1, 1, 0],
                      x: '-50%',
                      y: -38,
                      scale: [0.65, 1.18, 1, 1],
                      // Softens as it rises, so it reads as dissipating rather
                      // than simply switching off at the end.
                      filter: ['blur(0px)', 'blur(0px)', 'blur(0px)', 'blur(2px)'],
                    }}
                    transition={{
                      duration: 1.45,
                      ease: [0.16, 1, 0.3, 1],
                      times: [0, 0.16, 0.55, 1],
                    }}
                    onAnimationComplete={clearFlash}
                  >
                    +{flash.amount}
                  </motion.span>
                )}
              </AnimatePresence>
              {/* Two amber pulses on the icon itself.
                  Driven by `filter`, not by text colour, because this is an
                  `<img>` whenever the channel has its own points art and colour
                  would do nothing to it. A drop-shadow in amber plus a
                  brightness lift reads as a flash on both the image and the
                  fallback glyph. Two beats, not one: a single pulse at this size
                  is easy to miss entirely, and more than two starts nagging. */}
              <motion.span
                className="flex items-center justify-center"
                animate={
                  flash
                    ? {
                        scale: [1, 1.3, 1, 1.3, 1],
                        filter: [
                          'drop-shadow(0 0 0 rgba(234,179,8,0)) brightness(1)',
                          'drop-shadow(0 0 7px rgba(234,179,8,0.95)) brightness(1.55)',
                          'drop-shadow(0 0 0 rgba(234,179,8,0)) brightness(1)',
                          'drop-shadow(0 0 7px rgba(234,179,8,0.95)) brightness(1.55)',
                          'drop-shadow(0 0 0 rgba(234,179,8,0)) brightness(1)',
                        ],
                      }
                    : { scale: 1 }
                }
                transition={{
                  duration: 1.15,
                  ease: [0.2, 0.8, 0.2, 1],
                  times: [0, 0.22, 0.45, 0.67, 1],
                }}
              >
                {pointsOpen ? (
                  <X size={19} weight="bold" />
                ) : points.availableClaimId ? (
                  // A chest waiting (auto-collect off, or a room the player is
                  // not on): the button becomes the chest, like the desktop's.
                  // Tapping it collects the chest.
                  <span className="chest-attention flex">
                    <WalletChestGlyph size={19} />
                  </span>
                ) : points.balance === null && walletBits?.gem_url ? (
                  // Bits only: Twitch's still gem leads.
                  <img src={walletBits.gem_url} alt="" className="w-[19px] h-[19px] object-contain" draggable={false} />
                ) : walletBits?.gem_url ? (
                  // Bits live here too: points and the gem share one split glyph.
                  <SplitWalletGlyph points={pointsArt} pointsIcon={points.iconUrl} gemUrl={walletBits.gem_url} />
                ) : (
                  pointsArt(19)
                )}
              </motion.span>
              {/* Points, then Bits, in the glyph's own order: the glyph is the key. */}
              {shownBalances.map((balance, i) => (
                <Fragment key={i}>
                  {i > 0 && <span className="h-3 w-px bg-textSecondary/35" aria-hidden="true" />}
                  <span className="text-[12px] font-semibold tabular-nums leading-none whitespace-nowrap text-textPrimary">
                    {formatShortCount(balance)}
                  </span>
                </Fragment>
              ))}
            </button>
          )}
        </div>

        {/* The bar's right end, one control with every job it has: Send the
            moment the field holds a message, whatever sits here otherwise (the
            chat menu, or the inbox tray when Settings swaps the pair), the way
            to put the keyboard away while you type with nothing written
            (scrolling the chat never does), and the way back to it while the
            emote panel holds its place. One element in one spot: the marks
            crossfade and the accent fill grows in under Send, so nothing slides
            in or moves. Send is always on the right. */}
        <button
          // Held on the press while typing, so the field keeps the keyboard
          // until the tap lands (and after a send, for the next message).
          onPointerDown={(e) => {
            if (typing) e.preventDefault();
          }}
          onClick={() => {
            if (endMode === 'send') {
              void send();
            } else if (endMode === 'keyboardUp') {
              backToKeyboard();
            } else if (endMode === 'keyboard') {
              inputRef.current?.blur();
            } else if (endMode === 'tray') {
              toggleInboxSheet(inboxSurface);
            } else {
              openMenu();
            }
          }}
          // Held while slow mode makes you wait (the button counts it down)
          // and while a cheer in the message has a problem.
          disabled={endMode === 'send' && (sending || slowWait > 0 || cheerBlocked)}
          data-inbox-tray={rightEnd === 'tray' ? '' : undefined}
          style={{ order: 2 }}
          className={`chrome-glaze chrome-glaze--control relative shrink-0 flex items-center justify-center self-end w-10 h-10 transition-opacity duration-200 ${
            endMode === 'send' ? 'sn-send-ink' : inboxOpenHere && endMode === 'tray' ? 'text-textPrimary' : 'text-textSecondary active:text-textPrimary'
          } ${endMode === 'send' && (sending || cheerBlocked) ? 'opacity-50' : ''}`}
          aria-label={
            endMode === 'send'
              ? slowWait > 0
                ? `Send again in ${slowWait} seconds`
                : composerCheer
                  ? `Cheer ${composerCheer.total} bits`
                  : 'Send'
              : endMode === 'keyboardUp'
                ? 'Show keyboard'
                : endMode === 'keyboard'
                ? 'Hide keyboard'
                : endMode === 'tray'
                  ? inboxOpenHere
                    ? 'Close the inbox'
                    : inboxUnread > 0
                      ? `Inbox, ${inboxUnread} new`
                      : 'Inbox'
                  : trayFolded && inboxUnread > 0
                    ? `Chat options, ${inboxUnread} new in the inbox`
                    : 'Chat options'
          }
        >
          {/* Send's accent fill, grown in under its mark (opacity and scale
              only, so it holds the frame rate). */}
          <span
            aria-hidden
            className="absolute inset-0 rounded-full bg-accent pointer-events-none"
            style={{
              opacity: endMode === 'send' ? 1 : 0,
              transform: `scale(${endMode === 'send' ? 1 : 0.55})`,
              transition: 'opacity 160ms ease-out, transform 280ms cubic-bezier(0.2, 0.8, 0.2, 1)',
            }}
          />
          {/* Every mark in one place, crossfading, so nothing moves. */}
          <span className={`${END_MARK} ${endMode === 'menu' ? 'opacity-100' : 'opacity-0'}`}>
            <DotsThreeVertical size={22} weight="bold" />
          </span>
          {rightEnd === 'tray' && (
            <span className={`${END_MARK} ${endMode === 'tray' ? 'opacity-100' : 'opacity-0'}`}>
              {inboxOpenHere ? <XIcon size={19} strokeWidth={2.25} /> : <InboxIcon size={19} strokeWidth={2} />}
            </span>
          )}
          <span className={`${END_MARK} ${endMode === 'keyboard' ? 'opacity-100' : 'opacity-0'}`}>
            <KeyboardHideIcon size={20} />
          </span>
          <span className={`${END_MARK} ${endMode === 'keyboardUp' ? 'opacity-100' : 'opacity-0'}`}>
            <KeyboardShowIcon size={20} />
          </span>
          <span className={`${END_MARK} ${endMode === 'send' ? 'opacity-100' : 'opacity-0'}`}>
            {slowWait > 0 ? (
              <span className="text-[14px] font-semibold tabular-nums">{slowWait}</span>
            ) : composerCheer?.icon_url ? (
              // The message cheers: Send spends, so it wears the gem.
              <img src={composerCheer.icon_url} alt="" className="h-5 w-5 object-contain" draggable={false} />
            ) : (
              <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M12 19l9 2-9-18-9 18 9-2zm0 0v-8"
                />
              </svg>
            )}
          </span>
          {/* The inbox's count rides the corner beside the field: on the tray
              when it sits here, or on the chat menu when a narrow bar folded
              the tray into it. Not while the end is Send or the keyboard's. */}
          {(endMode === 'tray' ? !inboxOpenHere : endMode === 'menu' && trayFolded) && inboxUnread > 0 && (
            <span className="sn-inbox-badge absolute -top-1 -left-1 min-w-[17px] h-[17px] px-[5px] rounded-full text-[10.5px] font-bold leading-[17px] text-center tabular-nums pointer-events-none">
              {fitCount(inboxUnread)}
            </span>
          )}
        </button>

        {/* While the message cheers: what Send will spend, live, and a way
            to take the cheer out. Send is the yes. */}
        {isTwitch && (
          <CheerBar channelLogin={channel ?? undefined} channelId={channelId ?? undefined} text={text} onSetText={setText} />
        )}

      </div>

      {/* The emote menu in the keyboard's place, at its height, and never
          shorter than one you can browse. */}
      <div
        className="overflow-hidden"
        style={{ height: emotesOpen ? emotePanelHeight(keyboardHeight) : 0, marginTop: emotesOpen ? 8 : 0 }}
        onTouchStart={(e) => {
          const top = e.currentTarget.getBoundingClientRect().top;
          const t = e.touches[0];
          panelSwipe.current = t && t.clientY - top < 96 ? t.clientY : null;
        }}
        onTouchMove={(e) => {
          const t = e.touches[0];
          if (panelSwipe.current != null && t && t.clientY - panelSwipe.current > 56) {
            panelSwipe.current = null;
            setEmotesOpen(false);
          }
        }}
        onTouchEnd={() => {
          panelSwipe.current = null;
        }}
      >
        <EmotePickerPanel
          open={emotesOpen}
          onClose={() => setEmotesOpen(false)}
          ignoreRef={composerRef}
          emotes={emotes}
          isTwitch={isTwitch}
          isKick={provider === 'kick'}
          channelId={channelId ?? undefined}
          channelLogin={channel ?? undefined}
          isLoadingEmotes={!emotes}
          onInsert={insertEmote}
          images={provider !== 'tiktok'}
          className={PANEL_CLASS}
        />
      </div>

      {/* The rewards, as a sheet: the phone's own surface for a menu like this.
          Its first open does not wait on Twitch: the read below fills Rust's
          list as soon as there is a points button, and the menu paints that
          list at once while it reads a fresh one. */}
      {hasWallet && channel && channelId && (
        <MobileSheet open={pointsOpen} onClose={() => setPointsOpen(false)} maxHeightFraction={0.78}>
          <WalletSheet
            channelLogin={channel}
            channelId={channelId}
            points={
              points.balance !== null
                ? { balance: points.balance, name: points.name, iconUrl: points.iconUrl, loading: false, lastGain: null }
                : null
            }
            bits={walletBits}
            onClose={() => setPointsOpen(false)}
            onPointsRefresh={points.refresh}
            onEmotesChange={() => {
              // Redeeming an emote-unlock reward changes the picker's contents.
              void refreshChannelEmotes(channel, channelId);
            }}
            onInsert={insertEmote}
          />
        </MobileSheet>
      )}
      <ComposerMenuSheet
        open={menuOpen}
        onClose={() => setMenuOpen(false)}
        activeChannel={channel}
        provider={provider}
        activeLabel={channelLabel}
        channelId={channelId}
        isModerator={isModerator}
        roomModes={gating.view ?? null}
        modToolsOn={modToolsOn}
        onToggleModTools={onToggleModTools}
        onAddChat={onAddChat}
        onReload={onReload}
        onBrowseCommands={channel && isTwitch ? () => setCommandsOpen(true) : undefined}
        onManageChannel={channel && channelId && isTwitch && isModerator ? () => setManageOpen(true) : undefined}
        onModeration={channel && channelId && isTwitch && isModerator ? () => setModerationOpen(true) : undefined}
        onShowPinned={hiddenPinWaiting ? () => pins.setOpen(true) : undefined}
        onCloseChat={onCloseChat}
        inbox={trayFolded ? { unread: inboxUnread, open: () => toggleInboxSheet(inboxSurface) } : undefined}
      />
      {channel && channelId && isTwitch && isModerator && (
        <ModerationSheet
          open={moderationOpen}
          onClose={() => setModerationOpen(false)}
          channelId={channelId}
          channelLogin={channel}
          onManageChannel={() => setTimeout(() => setManageOpen(true), 260)}
        />
      )}
      {channel && channelId && isTwitch && isModerator && (
        <ManageChannelSheet
          open={manageOpen}
          onClose={() => setManageOpen(false)}
          channelId={channelId}
          channelLogin={channel}
        />
      )}
      <MobileCommandSheet
        open={commandsOpen}
        onClose={() => setCommandsOpen(false)}
        channel={channel ?? ''}
        onPick={(cmd) => {
          setText(`/${cmd.name} `);
          inputRef.current?.focus();
        }}
      />
    </div>
  );
};

export const MobileChatInput = React.memo(MobileChatInputImpl);
