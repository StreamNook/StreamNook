// The menu behind the composer's trailing button when there is nothing to send.
// Send and this share one slot, so the composer never carries a dead button.
import React from 'react';
import { ArrowsClockwise, ChatsCircle, Heart, HeartBreak, ShieldCheck, Star, Terminal, X } from 'phosphor-react';
import {
  Fingerprint as ModeUnique,
  Heart as ModeFollowers,
  Smile as ModeEmotes,
  Star as ModeSubs,
  Timer as ModeSlow,
  type LucideIcon,
} from 'lucide-react';
import { MobileSheet } from '../ui/MobileSheet';
import { useChannelSocial } from '../../hooks/useChannelSocial';
import { platformTerms } from '../../utils/platformTerms';
import type { ProviderId } from '../../types/providers';
import type { RoomModeKind, RoomModeView } from '../../stores/chatConnectionStore';

// The same glyph per mode as the composer's room-mode icon, so the icon you
// tapped is the one you find named here.
const MODE_ICONS: Record<RoomModeKind, LucideIcon> = {
  subs_only: ModeSubs,
  emote_only: ModeEmotes,
  followers_only: ModeFollowers,
  slow: ModeSlow,
  unique: ModeUnique,
};

interface Props {
  open: boolean;
  onClose: () => void;
  /** Null when no chat is open. The bare login / slug the platform uses. */
  activeChannel: string | null;
  /** Platform the open chat is on. */
  provider: ProviderId;
  activeLabel: string | null;
  /** Broadcaster id, for follow/unfollow. Null when no chat is open. */
  channelId: string | null;
  isModerator: boolean;
  /** The room's modes in Rust's words, shown as context. Null when the room
   *  has none. */
  roomModes: RoomModeView | null;
  modToolsOn: boolean;
  onToggleModTools: () => void;
  onAddChat: () => void;
  onReload: () => void;
  /** Opens the chat command browser. Absent when no chat is open. */
  onBrowseCommands?: () => void;
  /** Absent for the stream-following tab, which is not closable. */
  onCloseChat?: () => void;
}

export const ComposerMenuSheet: React.FC<Props> = ({
  open,
  onClose,
  activeChannel,
  provider,
  activeLabel,
  channelId,
  isModerator,
  roomModes,
  modToolsOn,
  onToggleModTools,
  onAddChat,
  onReload,
  onBrowseCommands,
  onCloseChat,
}) => {
  const row =
    'sn-touch flex items-center gap-3 px-2 text-[15px] text-textPrimary active:opacity-70 disabled:opacity-45';

  // The desktop client's follow logic, ported rather than reimplemented: this
  // hook lives in hooks/ precisely so several surfaces can share it, and it
  // already backs the desktop player overlay and MultiNook tiles.
  //
  // `enabled: open` matters. The sheet is always mounted, and the hook also
  // checks subscription status on mount, so running it unconditionally would
  // fire those lookups for every chat all the time. That flag exists for exactly
  // this (MultiNook runs it only for the focused tile).
  //
  // Its subscribe half opens the platform's own checkout in the login overlay.
  // Follow stays Twitch-only here (it needs the Twitch channel id); subscribe
  // works for Kick rooms too, whose state comes from the Kick account sync.
  const {
    isFollowing,
    followLoading,
    handleFollowClick,
    isSubscribed,
    hasSubHistory,
    subscriberBadgeUrl,
    handleSubscribeClick,
  } = useChannelSocial({
    provider,
    userId: channelId,
    userLogin: activeChannel,
    userName: activeLabel,
    enabled: open && !!activeChannel,
  });
  const terms = platformTerms(provider);
  const canSubscribe = !!activeChannel && (provider === 'twitch' || provider === 'kick');

  return (
    <MobileSheet open={open} onClose={onClose} title={activeLabel ?? 'Chat'} maxHeightFraction={0.5}>
      <div className="flex flex-col">
        {/* Room state lives here rather than above the composer. Followers-only
            while you can still talk is context worth having, not a warning worth
            a permanent line over chat. Status, not actions: one quiet block, its
            lines closer together than the rows below, so it never reads as a
            set of buttons. Each mode carries the composer's glyph for it, its
            name and what it means; the footer says where you stand when every
            mode spares you. */}
        {roomModes && roomModes.modes.length > 0 && (
          <div className="px-2 pb-3 mb-1 border-b border-borderSubtle">
            <div className="text-[12px] font-semibold uppercase tracking-wide text-textMuted mb-2">
              This chat is in
            </div>
            <div className="flex flex-col gap-2">
              {roomModes.modes.map((mode) => {
                const Icon = MODE_ICONS[mode.kind];
                return (
                  <div key={mode.kind} className="flex items-start gap-3">
                    <span className="flex w-[19px] h-[19px] shrink-0 items-center justify-center" aria-hidden>
                      <Icon size={14} strokeWidth={2.2} className="text-textMuted" />
                    </span>
                    <p className="flex-1 min-w-0 text-[13.5px] leading-[19px]">
                      <span className="font-medium text-textSecondary">{mode.compact}</span>{' '}
                      <span className="text-textMuted">{mode.detail}</span>
                    </p>
                  </div>
                );
              })}
            </div>
            {roomModes.standing && (
              <div className="mt-2 pl-[31px] text-[12.5px] leading-snug text-textSecondary">
                {roomModes.standing}
              </div>
            )}
          </div>
        )}
        {/* Follow / unfollow. Green heart to follow, red broken heart to
            unfollow, matching the desktop overlay's Heart / HeartBreak pair.
            Held back until the status is known (`isFollowing` is null while
            checking) rather than guessing a label and flipping it a moment
            later. The sheet stays open on tap: the row reflects the new state
            in place, which is the confirmation. */}
        {channelId && isFollowing !== null && (
          <button
            onClick={() => void handleFollowClick()}
            disabled={followLoading}
            className={`${row} ${isFollowing ? '!text-error' : '!text-success'}`}
          >
            {isFollowing ? (
              <HeartBreak size={19} weight="fill" className="shrink-0" />
            ) : (
              <Heart size={19} weight="fill" className="shrink-0" />
            )}
            <span className="flex-1 text-left">
              {isFollowing ? 'Unfollow' : 'Follow'}
              {activeLabel ? ` ${activeLabel}` : ''}
            </span>
          </button>
        )}

        {/* Subscribe, or resubscribe, or gift once subscribed: the same three
            states the desktop control shows, in the platform's own words. The
            sheet closes because the checkout opens over it. */}
        {canSubscribe && (
          <button
            onClick={() => {
              onClose();
              void handleSubscribeClick();
            }}
            className={row}
          >
            {subscriberBadgeUrl ? (
              <img src={subscriberBadgeUrl} alt="" draggable={false} className="w-[19px] h-[19px] shrink-0" />
            ) : (
              <Star size={19} weight="fill" className="text-accent shrink-0" />
            )}
            <span className="flex-1 text-left">
              {isSubscribed ? terms.paidGift : hasSubHistory ? terms.paidAgain : terms.paid}
              {!isSubscribed && activeLabel ? ` to ${activeLabel}` : ''}
            </span>
            {isSubscribed && <span className="text-[12.5px] text-textMuted">Subscribed</span>}
          </button>
        )}

        <button
          onClick={() => {
            onAddChat();
            onClose();
          }}
          className={row}
        >
          <ChatsCircle size={19} className="text-textSecondary shrink-0" />
          <span className="flex-1 text-left">Add a chat</span>
        </button>

        <button
          onClick={() => {
            onReload();
            onClose();
          }}
          disabled={!activeChannel}
          className={row}
        >
          <ArrowsClockwise size={19} className="text-textSecondary shrink-0" />
          <span className="flex-1 text-left">Reload this chat</span>
        </button>

        {onBrowseCommands && (
          <button
            onClick={() => {
              onClose();
              onBrowseCommands();
            }}
            className={row}
          >
            <Terminal size={19} className="text-textSecondary shrink-0" />
            <span className="flex-1 text-left">Chat commands</span>
          </button>
        )}

        {/* Only offered where the actions would actually succeed: Helix rejects
            them without mod powers in this specific channel. */}
        {isModerator && (
          <button onClick={onToggleModTools} className={row}>
            <ShieldCheck
              size={19}
              weight={modToolsOn ? 'fill' : 'regular'}
              className={modToolsOn ? 'text-accent shrink-0' : 'text-textSecondary shrink-0'}
            />
            <span className="flex-1 text-left">Moderator tools</span>
            <span className="text-[12.5px] text-textMuted">{modToolsOn ? 'On' : 'Off'}</span>
          </button>
        )}

        {onCloseChat && (
          <button
            onClick={() => {
              onCloseChat();
              onClose();
            }}
            className={`${row} !text-error`}
          >
            <X size={19} className="shrink-0" />
            <span className="flex-1 text-left">Close this chat</span>
          </button>
        )}

        {isModerator && modToolsOn && (
          <div className="mt-2 px-2 text-[12.5px] text-textMuted leading-snug">
            Long-press any message for delete, timeout and ban.
          </div>
        )}
      </div>
    </MobileSheet>
  );
};
