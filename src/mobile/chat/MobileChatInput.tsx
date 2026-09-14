// Lean touch chat composer: text send, the shared emote/emoji picker, channel
// points, and a menu that shares the trailing slot with Send. Deliberately drops
// the desktop input's slash commands, user-command expansion, resub/streak
// modes, send-as switching, and arrow history; those are desktop affordances.
//
// The picker and the points menu are the SAME components the desktop composer
// uses. Both are prop-driven, carry no window/hover dependencies, and anchor
// themselves with `absolute bottom-full` — so they only need a `relative`
// ancestor here, and mobile gets provider tabs, favorites, emoji and search for
// free instead of a second, worse implementation.
//
// Channel comes in as a prop rather than being read off `currentStream`: with
// several chat tabs open the composer must target the tab you are looking at,
// which is not necessarily the stream playing.
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { listen } from '@tauri-apps/api/event';
import { DotsThreeVertical, X } from 'phosphor-react';
import { useAppStore } from '../../stores/AppStore';
import { incrementStat } from '../../services/supabaseService';
import { refreshChannelEmotes, sendChannelMessage } from '../../stores/chatConnectionStore';
import type { EmoteSet } from '../../services/emoteService';
import { Logger } from '../../utils/logger';
import { EmotePickerPanel, useSwappingSmiley } from '../../components/chat/EmotePickerPanel';
import ChannelPointsMenu from '../../components/ChannelPointsMenu';
import { ChannelPointsIcon } from '../../components/ChannelPointsIcon';
import { useChannelPoints } from './useChannelPoints';
import { MobileEmoteCarousel } from './MobileEmoteCarousel';
import { matchEmoteTokens } from './emoteTabMatch';
import { getWordRange } from '../../utils/chatInputWord';
import { useEmoteOwnerNames } from './useEmoteOwnerNames';
import { ComposerMenuSheet } from './ComposerMenuSheet';
import { StreakBanners } from './StreakBanners';
import type { ChatGating } from './chatGating';

// Shorter than the desktop's 520px so the panel still clears the soft keyboard,
// and opaque rather than the panel's own 95%. That 5% reads as solid over a
// desktop-sized popover but lets a whole screen of chat show through at phone
// size, which is most of what made the old picker hard to read. `!` is needed
// because the shared panel sets its background inline.
//
// Deliberately NOT solved with a backdrop blur: that gets stripped entirely
// when Glassiness is off, so it cannot be what keeps text legible, and a large
// blur is exactly the per-frame cost just removed from chat rows for phone GPUs.
//
// The height clamp is keyboard-aware on purpose. `vh` is the whole screen here
// (edge-to-edge means the soft keyboard does not resize the viewport, the native
// inset bridge reports it as --sn-kb instead), so a fixed 52vh panel grows
// straight up past the status bar once the keyboard opens.
const PANEL_CLASS =
  'absolute bottom-full left-0 right-0 mb-2 h-[52vh] max-h-[min(420px,calc(100dvh-var(--sn-kb,0px)-var(--sn-safe-t,0px)-140px))] border border-borderSubtle rounded-xl shadow-lg flex flex-col overflow-hidden origin-bottom z-50 !bg-background';

interface Props {
  channel: string | null;
  channelId: string | null;
  channelLabel: string | null;
  gating: ChatGating;
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
}

// Memoized below. The pane re-renders on every arriving chat message and the
// composer's props barely ever change, so without this the whole input subtree
// (and the emote work behind it) is rebuilt at chat's message rate. The memo is
// only worth anything while every callback prop stays referentially stable, so
// keep them in useCallback on the pane side.
const MobileChatInputImpl: React.FC<Props> = ({
  channel,
  channelId,
  channelLabel,
  gating,
  emotes,
  replyTo,
  onCancelReply,
  isModerator,
  modToolsOn,
  onToggleModTools,
  onAddChat,
  onReload,
  onCloseChat,
}) => {
  const currentUser = useAppStore((s) => s.currentUser);
  const chatInput = useAppStore((s) => s.settings.chat_input);
  const [text, setText] = useState('');
  const [emotesOpen, setEmotesOpen] = useState(false);
  const [pointsOpen, setPointsOpen] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const [sending, setSending] = useState(false);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const { currentSmiley, isSmileyTransitioning, cycleEmoteSmiley } = useSwappingSmiley();
  // Points follow the STREAM, not the focused tab, which is the opposite of
  // everything else in this composer.
  //
  // This hook is the only thing on the phone that collects the bonus chest, and
  // Twitch only ever offers a chest for the channel it believes you are
  // watching, which is the one the watch heartbeat is crediting. Pointed at a
  // second chat tab it would poll a channel that never has a chest, while the
  // watched channel's chest sat there and expired. Roughly fifty points every
  // quarter hour, silently.
  //
  // Desktop resolves it the same way and has multi-channel chat too: its
  // ChatWidget keys the points panel to currentStream, never to the focused
  // room. Sending stays on the tab; only the points readout moves.
  const pointsChannel = useAppStore((s) => s.currentStream?.user_login) ?? channel;
  const pointsChannelId = useAppStore((s) => s.currentStream?.user_id) ?? channelId;
  const points = useChannelPoints(pointsChannel, pointsChannelId);
  const emoteOwnerNames = useEmoteOwnerNames(emotes);

  // Points feedback.
  //
  // The event is `channel-points-earned` with a `points` field. It is NOT
  // `channel-points-claimed`, which nothing in the Rust source has ever emitted.
  //
  // Two sources feed this, and it needs both. The event covers collecting a
  // bonus chest, which is roughly a quarter-hour apart. Everything in between is
  // passive per-minute earning, which is genuinely happening but announces
  // itself nowhere, so `useChannelPoints` spots it by comparing successive reads
  // of the balance and reports it as a gain. On the event alone this would fire
  // about four times an hour and still read as broken.
  //
  // Note the event does not arrive from a background watcher on this platform.
  // The PubSub service points at an endpoint Twitch decommissioned, and the
  // background claim path belongs to the automation plugin, which does not run
  // here. On mobile the only emitter is the claim this composer's own hook
  // makes.
  const [pointsFlash, setPointsFlash] = useState<{ id: number; amount: number } | null>(null);
  const flashSeq = useRef(0);
  // Destructured because `points` is a fresh object literal every render, so
  // depending on it re-ran this effect on EVERY render: the Tauri listener was
  // torn down and re-registered constantly, and `listen` is async, so events
  // arriving during a swap had nothing subscribed. `refresh` is a stable
  // useCallback, which is what makes this hold still.
  const { refresh: refreshPoints } = points;
  useEffect(() => {
    let cancelled = false;
    const un = listen<{ points?: number; channel_id?: string }>(
      'channel-points-earned',
      (e) => {
        if (cancelled) return;
        const amount = e.payload?.points ?? 0;
        if (amount <= 0) return;
        // Points accrue on every channel you have collected from, not only the
        // one on screen, so ignore events for other channels or the number would
        // appear over the wrong room's balance.
        if (e.payload?.channel_id && channelId && e.payload.channel_id !== channelId) return;
        flashSeq.current += 1;
        setPointsFlash({ id: flashSeq.current, amount });
        // Pull the new balance so the menu is right if you open it straight after.
        refreshPoints();
      },
    );
    return () => {
      cancelled = true;
      void un.then((f) => f()).catch(() => {});
    };
  }, [refreshPoints, channelId]);

  // Whichever source raised a credit, one float. `gain` carries its own id, so
  // merging cannot make two separate credits look like one element.
  const flash = pointsFlash ?? points.gain;
  const clearFlash = () => {
    setPointsFlash(null);
    points.clearGain();
  };

  const canSend = !!text.trim();

  const send = async () => {
    const message = text.trim();
    if (!message || !currentUser || !channel || sending) return;
    setSending(true);
    try {
      await sendChannelMessage(
        channel,
        message,
        {
          username: currentUser.login || currentUser.username,
          displayName: currentUser.display_name || currentUser.username,
          userId: currentUser.user_id,
          color: undefined,
          badges: '',
        },
        replyTo?.messageId,
      );
      setText('');
      onCancelReply?.();
      incrementStat(currentUser.user_id, 'messages_sent', 1).catch((err) => {
        Logger.warn('[MobileChatInput] messages_sent stat failed:', err);
      });
    } catch (err) {
      Logger.error('[MobileChatInput] send failed:', err);
    } finally {
      setSending(false);
    }
  };

  // The shared picker hands back the literal token to insert, which is an emote
  // name for emotes and the character itself for emoji.
  const insertEmote = (token: string) => {
    setText((t) => (t.length === 0 || t.endsWith(' ') ? `${t}${token} ` : `${t} ${token} `));
    inputRef.current?.focus();
  };

  // Suggestions for the word being typed.
  //
  // The emote menu is a fine way to browse, and a poor way to reach one emote
  // you already know the name of: it covers most of a phone screen to insert a
  // few characters. Typing the first few letters and tapping is the fast path,
  // and it is the one the desktop has had all along behind a key this platform
  // does not have.
  const suggestions = useMemo(() => {
    // Completes the LAST word, rather than reading the caret. On a phone the
    // caret is essentially always at the end, and the real position would not
    // re-run this anyway: moving it changes no state. A trailing space falls out
    // of this correctly, since the word at the end is then empty.
    const [start, end] = getWordRange(text, text.length);
    const word = text.slice(start, end);
    // One character matches most of the set and is noise, not help.
    if (word.length < 2) return [];
    return matchEmoteTokens(word, emotes, {
      mode: chatInput?.emote_tab_complete_match_mode,
      includeChatters: chatInput?.emote_tab_complete_include_chatters,
    });
  }, [text, emotes, chatInput]);

  const showSuggestions =
    (chatInput?.emote_tab_complete_enabled ?? true) &&
    !emotesOpen &&
    !pointsOpen &&
    suggestions.length > 0;

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
        message={text}
        onShared={() => setText('')}
      />
      {/* No warning line here, deliberately. It rendered `gating.reason`, which
          is the exact string the disabled field's placeholder already shows, in
          the same warning colour -- the same sentence twice, stacked. The
          placeholder alone says why the field is dead, and it says it in the
          field you were about to type in. The chat menu carries the room's full
          state, and now the Follow action alongside it. */}
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
              onSelect={(tok) => completeWith(tok.name)}
            />
          )}
        </AnimatePresence>
        {/* The emote trigger and the points button live INSIDE the field so the
            typing area keeps full width instead of being squeezed by chrome. */}
        <div className="glass-input flex-1 min-w-0 flex items-end gap-1 pl-1 pr-2">
          <button
            onClick={() => {
              cycleEmoteSmiley();
              setPointsOpen(false);
              setEmotesOpen((v) => !v);
            }}
            className={`shrink-0 w-9 h-9 flex items-center justify-center text-[20px] leading-none active:opacity-70 ${
              emotesOpen ? 'text-accent' : ''
            }`}
            aria-label={emotesOpen ? 'Close emotes' : 'Emotes and emoji'}
          >
            {/* Becomes an X while open, so the button that opened the panel is
                visibly the one that closes it. Same swap the desktop composer
                does. */}
            {emotesOpen ? (
              <X size={19} weight="bold" />
            ) : (
              <span
                className="transition-[opacity,transform] duration-100"
                style={{
                  opacity: isSmileyTransitioning ? 0 : 1,
                  transform: isSmileyTransitioning ? 'scale(0.8)' : 'scale(1)',
                }}
              >
                {currentSmiley}
              </span>
            )}
          </button>
          <textarea
            ref={inputRef}
            value={text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault();
                void send();
              }
            }}
            placeholder={gating.blocked ? (gating.reason ?? 'Chat restricted') : 'Send a message'}
            // Typing is switched off, not just discouraged, when the badges prove
            // the send would be rejected. The amber placeholder says why.
            disabled={!!gating.blocked}
            rows={1}
            className={`flex-1 min-w-0 resize-none bg-transparent py-2 text-[15px] leading-[1.4] text-textPrimary outline-none max-h-[96px] ${
              gating.blocked
                ? 'placeholder:text-warning placeholder:font-medium'
                : 'placeholder:text-textMuted'
            }`}
            enterKeyHint="send"
          />
          {/* Icon only. The balance was briefly shown here to prove earning was
              happening, and once proven it went back: it cost the message field
              real width to state a number nobody needs at rest, and the menu has
              always printed it in full. What matters is the MOMENT of earning,
              which the flash and the floating amount carry. */}
          {points.balance !== null && channel && (
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
              className={`relative shrink-0 w-9 h-9 flex items-center justify-center active:opacity-70 ${
                pointsOpen ? 'text-accent' : 'text-textSecondary'
              }`}
              aria-label={pointsOpen ? 'Close channel points' : points.name || 'Channel points'}
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
              {/* A chest waiting to be collected. Only ever shows with
                  auto-collect turned off, since otherwise it is taken the moment
                  it appears. Tapping the button collects it. */}
              {points.availableClaimId && (
                <span className="absolute top-1 right-1 w-2 h-2 rounded-full bg-accent" />
              )}
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
                ) : points.iconUrl ? (
                  <img
                    src={points.iconUrl}
                    alt=""
                    className="w-[19px] h-[19px] object-contain"
                    draggable={false}
                  />
                ) : (
                  <ChannelPointsIcon size={19} />
                )}
              </motion.span>
            </button>
          )}
        </div>

        {/* One slot, two jobs: Send while there is a message, otherwise the chat
            menu. Avoids parking a permanently disabled Send on screen. */}
        {canSend ? (
          <button
            onClick={() => void send()}
            disabled={sending}
            className="glass-button shrink-0 flex items-center justify-center self-end w-10 h-10 text-white rounded transition-all duration-300 disabled:opacity-50"
            aria-label="Send"
          >
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M12 19l9 2-9-18-9 18 9-2zm0 0v-8"
              />
            </svg>
          </button>
        ) : (
          <button
            onClick={() => {
              setEmotesOpen(false);
              setPointsOpen(false);
              setMenuOpen(true);
            }}
            className="glass-button shrink-0 flex items-center justify-center self-end w-10 h-10 rounded text-textSecondary active:text-textPrimary transition-all duration-300"
            aria-label="Chat options"
          >
            <DotsThreeVertical size={20} weight="bold" />
          </button>
        )}

        <EmotePickerPanel
          open={emotesOpen}
          onClose={() => setEmotesOpen(false)}
          emotes={emotes}
          isTwitch
          isKick={false}
          channelId={channelId ?? undefined}
          channelLogin={channel ?? undefined}
          isLoadingEmotes={!emotes}
          channelNameCache={emoteOwnerNames}
          onInsert={insertEmote}
          className={PANEL_CLASS}
        />
        {/* Mounted as soon as there is a points button, not on tap.
            Opening used to flash a spinner: the menu fetches its rewards on
            mount, that is a GQL round trip with no backend cache, and mounting
            only when tapped put the whole trip inside the open. Mounting it up
            front spends the request while nobody is waiting on it, so the tap
            reveals a list that is already there.

            Safe to mount early because the only work it does on mount is that
            one fetch plus an Escape listener; everything else (emote lists,
            unlockables) is triggered by interaction. The cost is one rewards
            request per channel whether or not the menu gets opened, which is a
            fair trade for the primary action on this bar never stuttering.

            Hidden with opacity and visibility rather than `display`, so the
            reveal can transition. NO transform on this wrapper: the menu
            positions itself `absolute bottom-full` against the composer, and a
            transform here would make this element its containing block and
            throw it out of place. */}
        {channel && channelId && points.balance !== null && (
          <div
            className={`transition-[opacity] duration-150 ease-out ${
              pointsOpen ? 'opacity-100' : 'opacity-0 invisible pointer-events-none'
            }`}
            aria-hidden={!pointsOpen}
          >
            <ChannelPointsMenu
              channelLogin={channel}
              channelId={channelId}
              currentBalance={points.balance}
              customPointsName={points.name}
              customPointsIconUrl={points.iconUrl}
              onClose={() => setPointsOpen(false)}
              onBalanceUpdate={points.refresh}
              onEmotesChange={() => {
                // Redeeming an emote-unlock reward changes the picker's contents.
                void refreshChannelEmotes(channel, channelId);
              }}
            />
          </div>
        )}
      </div>

      <ComposerMenuSheet
        open={menuOpen}
        onClose={() => setMenuOpen(false)}
        activeChannel={channel}
        activeLabel={channelLabel}
        channelId={channelId}
        isModerator={isModerator}
        gatingLabels={gating.labels}
        modToolsOn={modToolsOn}
        onToggleModTools={onToggleModTools}
        onAddChat={onAddChat}
        onReload={onReload}
        onCloseChat={onCloseChat}
      />
    </div>
  );
};

export const MobileChatInput = React.memo(MobileChatInputImpl);
