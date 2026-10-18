// The menu behind the composer's trailing button when there is nothing to send.
// Send and this share one slot, so the composer never carries a dead button.
import React, { useState } from 'react';
import { ArrowsClockwise, CaretRight, ChatsCircle, Heart, HeartBreak, PencilSimpleLine, ShieldCheck, Star, Terminal, X } from 'phosphor-react';
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
import { PROVIDERS, type ProviderId } from '../../types/providers';
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

// Every action row's glyph: phosphor's outline weight, one size, centered in
// the same 24px box, so no row's icon out-pulls another.
const ICON = 22;

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
  /** Opens the stream manager. Absent where there is nothing to manage
   *  (not a Twitch chat you moderate or own). */
  onManageChannel?: () => void;
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
  onManageChannel,
  onCloseChat,
}) => {
  const row =
    'sn-touch flex items-center gap-3 px-2 text-[15px] text-textPrimary active:opacity-70 disabled:opacity-45';
  const iconBox = 'flex w-6 h-6 shrink-0 items-center justify-center';

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

  // Follow sits at the top; Unfollow at the bottom with the other row that
  // takes something away. Where the row sits is decided once per opening, so
  // a tap flips its label in place (the confirmation) instead of moving it.
  const [lastOpen, setLastOpen] = useState(open);
  const [followSlot, setFollowSlot] = useState<'top' | 'bottom' | null>(null);
  if (open !== lastOpen) {
    setLastOpen(open);
    if (open) setFollowSlot(null);
  }
  if (open && followSlot === null && isFollowing !== null) setFollowSlot(isFollowing ? 'bottom' : 'top');
  // Held back until the status is known (`isFollowing` is null while
  // checking) rather than guessing a label and flipping it a moment later.
  // The sheet stays open on tap.
  const followRow =
    channelId && isFollowing !== null && followSlot !== null ? (
      <button
        onClick={() => void handleFollowClick()}
        disabled={followLoading}
        className={`${row} ${isFollowing ? '!text-error' : ''}`}
      >
        <span className={iconBox}>
          {isFollowing ? <HeartBreak size={ICON} /> : <Heart size={ICON} className="text-success" />}
        </span>
        <span className="flex-1 text-left">
          {isFollowing ? 'Unfollow' : 'Follow'}
          {activeLabel ? ` ${activeLabel}` : ''}
        </span>
      </button>
    ) : null;

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
                    <p className="flex-1 min-w-0 text-[13px] leading-[19px]">
                      <span className="font-medium text-textSecondary">{mode.compact}</span>{' '}
                      <span className="text-textMuted">{mode.detail}</span>
                    </p>
                  </div>
                );
              })}
            </div>
            {roomModes.standing && (
              <div className="mt-2 pl-[31px] text-[12px] leading-snug text-textSecondary">
                {roomModes.standing}
              </div>
            )}
          </div>
        )}
        {/* Follow, at the top when you do not follow yet. */}
        {followRow && followSlot === 'top' && followRow}

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
            <span className={iconBox}>
              {subscriberBadgeUrl ? (
                <img src={subscriberBadgeUrl} alt="" draggable={false} className="w-[22px] h-[22px]" />
              ) : (
                <Star size={ICON} style={{ color: PROVIDERS[provider].color }} />
              )}
            </span>
            <span className="flex-1 text-left">
              {isSubscribed ? terms.paidGift : hasSubHistory ? terms.paidAgain : terms.paid}
              {!isSubscribed && activeLabel ? ` to ${activeLabel}` : ''}
            </span>
            {isSubscribed && <span className="text-[12px] text-textMuted">Subscribed</span>}
          </button>
        )}

        <button
          onClick={() => {
            onAddChat();
            onClose();
          }}
          className={row}
        >
          <span className={iconBox}>
            <ChatsCircle size={ICON} className="text-textSecondary" />
          </span>
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
          <span className={iconBox}>
            <ArrowsClockwise size={ICON} className="text-textSecondary" />
          </span>
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
            <span className={iconBox}>
              <Terminal size={ICON} className="text-textSecondary" />
            </span>
            <span className="flex-1 text-left">Chat commands</span>
          </button>
        )}

        {onManageChannel && (
          <button
            onClick={() => {
              onClose();
              onManageChannel();
            }}
            className={row}
          >
            <span className={iconBox}>
              <PencilSimpleLine size={ICON} className="text-textSecondary" />
            </span>
            <span className="flex-1 text-left">Manage channel</span>
            <CaretRight size={16} className="text-textMuted" />
          </button>
        )}

        {/* Only offered where the actions would actually succeed: Helix rejects
            them without mod powers in this specific channel. On or off reads
            from the color and the trailing word; the glyph keeps its weight. */}
        {isModerator && (
          <button onClick={onToggleModTools} className={row}>
            <span className={iconBox}>
              <ShieldCheck size={ICON} className={modToolsOn ? 'text-accent' : 'text-textSecondary'} />
            </span>
            <span className="flex-1 text-left">Moderator tools</span>
            <span className="text-[12px] text-textMuted">{modToolsOn ? 'On' : 'Off'}</span>
          </button>
        )}

        {isModerator && modToolsOn && (
          <div className="px-2 pb-1 text-[12px] text-textMuted leading-snug">
            Long-press any message for delete, timeout and ban.
          </div>
        )}

        {/* The rows that take something away, last and set apart. */}
        {(onCloseChat || (followRow && followSlot === 'bottom')) && (
          <div className="my-1 mx-2 border-t border-borderSubtle" />
        )}

        {onCloseChat && (
          <button
            onClick={() => {
              onCloseChat();
              onClose();
            }}
            className={`${row} !text-error`}
          >
            <span className={iconBox}>
              <X size={ICON} />
            </span>
            <span className="flex-1 text-left">Close this chat</span>
          </button>
        )}

        {followRow && followSlot === 'bottom' && followRow}
      </div>
    </MobileSheet>
  );
};
