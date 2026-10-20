// The menu behind the composer's trailing button when there is nothing to send.
// Send and this share one slot, so the composer never carries a dead button.
import React, { useState } from 'react';
import { ArrowsClockwise, BookmarkSimple, Tray, CaretRight, ChatsCircle, Heart, HeartBreak, PencilSimpleLine, PushPin, ShieldCheck, Star, Terminal, X } from 'phosphor-react';
import {
  Fingerprint as ModeUnique,
  Heart as ModeFollowers,
  Smile as ModeEmotes,
  Star as ModeSubs,
  Timer as ModeSlow,
  type LucideIcon,
} from 'lucide-react';
import { MobileSheet } from '../ui/MobileSheet';
import { fitCount } from '../../utils/fitNumber';
import { GroupCard, GroupFooter, GroupLabel, GroupRow } from '../ui/GroupedList';
import { useChannelSocial } from '../../hooks/useChannelSocial';
import { platformTerms } from '../../utils/platformTerms';
import { PROVIDERS, type ProviderId } from '../../types/providers';
import type { RoomModeKind, RoomModeView } from '../../stores/chatConnectionStore';
import { useChatTabsStore } from './chatTabsStore';

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
  /** The room's moderation sheet (moderators, Twitch). */
  onModeration?: () => void;
  /** Opens the room's pin, folded away by Pinned Messages > Style: Hidden.
   *  Absent while no pin waits there. */
  onShowPinned?: () => void;
  /** Absent for the stream-following tab, which is not closable. */
  onCloseChat?: () => void;
  /** The inbox, while a narrow message bar has no room for its tray: its
   *  row and count live here instead. Absent while the tray is on the bar. */
  inbox?: { unread: number; open: () => void };
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
  onModeration,
  onShowPinned,
  onCloseChat,
  inbox,
}) => {
  // The stream's own chat closes with the player unless you keep it; every
  // other open chat is already one of yours.
  const streamTab = useChatTabsStore((s) => {
    const t = s.tabs.find((tab) => tab.channel === s.activeChannel);
    return t?.pinnedToStream ? t : null;
  });
  const keep = useChatTabsStore((s) => s.keep);

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
      <GroupRow
        onClick={() => void handleFollowClick()}
        disabled={followLoading}
        className={isFollowing ? '!text-error' : ''}
        icon={isFollowing ? <HeartBreak size={ICON} /> : <Heart size={ICON} className="text-success" />}
      >
        {isFollowing ? 'Unfollow' : 'Follow'}
        {activeLabel ? ` ${activeLabel}` : ''}
      </GroupRow>
    ) : null;

  return (
    <MobileSheet open={open} onClose={onClose} title={activeLabel ?? 'Chat'} maxHeightFraction={0.5}>
      {/* The same grouped pieces as the settings pages, as the iPhone menu
          draws them: the room's modes above, the actions on one card. */}
        {/* Room state lives here rather than above the composer. Followers-only
            while you can still talk is context worth having, not a warning worth
            a permanent line over chat. Status, not actions: one quiet block, its
            lines closer together than the rows below, so it never reads as a
            set of buttons. Each mode carries the composer's glyph for it, its
            name and what it means; the footer says where you stand when every
            mode spares you. */}
        {roomModes && roomModes.modes.length > 0 && (
          <div className="px-4 pb-3">
            <GroupLabel className="mb-2">This chat is in</GroupLabel>
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
      <GroupCard>
        {/* Follow, at the top when you do not follow yet. */}
        {followRow && followSlot === 'top' && followRow}

        {/* Subscribe, or resubscribe, or gift once subscribed: the same three
            states the desktop control shows, in the platform's own words. The
            sheet closes because the checkout opens over it. */}
        {canSubscribe && (
          <GroupRow
            onClick={() => {
              onClose();
              void handleSubscribeClick();
            }}
            icon={
              subscriberBadgeUrl ? (
                <img src={subscriberBadgeUrl} alt="" draggable={false} className="w-[22px] h-[22px]" />
              ) : (
                <Star size={ICON} style={{ color: PROVIDERS[provider].color }} />
              )
            }
            value={isSubscribed ? 'Subscribed' : undefined}
          >
            {isSubscribed ? terms.paidGift : hasSubHistory ? terms.paidAgain : terms.paid}
            {!isSubscribed && activeLabel ? ` to ${activeLabel}` : ''}
          </GroupRow>
        )}

        <GroupRow
          onClick={() => {
            onAddChat();
            onClose();
          }}
          icon={<ChatsCircle size={ICON} className="text-textSecondary" />}
        >
          Add a chat
        </GroupRow>

        {inbox && (
          <GroupRow
            onClick={() => {
              onClose();
              inbox.open();
            }}
            icon={<Tray size={ICON} className="text-textSecondary" />}
            trailing={
              inbox.unread > 0 ? (
                <span className="sn-inbox-badge mr-1 min-w-[18px] h-[18px] px-[5px] rounded-full text-[11px] font-bold leading-[18px] text-center tabular-nums">
                  {fitCount(inbox.unread)}
                </span>
              ) : undefined
            }
          >
            Inbox
          </GroupRow>
        )}

        {onShowPinned && (
          <GroupRow
            onClick={() => {
              onClose();
              onShowPinned();
            }}
            icon={<PushPin size={ICON} className="text-textSecondary" />}
          >
            Pinned message
          </GroupRow>
        )}

        <GroupRow
          onClick={() => {
            onReload();
            onClose();
          }}
          disabled={!activeChannel}
          icon={<ArrowsClockwise size={ICON} className="text-textSecondary" />}
        >
          Reload this chat
        </GroupRow>

        {streamTab && (
          <GroupRow
            onClick={() => keep(streamTab.channel, !streamTab.kept)}
            icon={
              <BookmarkSimple
                size={ICON}
                weight={streamTab.kept ? 'fill' : 'regular'}
                className={streamTab.kept ? 'text-accent' : 'text-textSecondary'}
              />
            }
            value={streamTab.kept ? 'On' : 'Off'}
          >
            Keep this chat
          </GroupRow>
        )}

        {onBrowseCommands && (
          <GroupRow
            onClick={() => {
              onClose();
              onBrowseCommands();
            }}
            icon={<Terminal size={ICON} className="text-textSecondary" />}
          >
            Chat commands
          </GroupRow>
        )}

        {onModeration && (
          <GroupRow
            onClick={() => {
              onClose();
              onModeration();
            }}
            icon={<ShieldCheck size={ICON} className="text-textSecondary" />}
            trailing={<CaretRight size={16} className="text-textMuted" />}
          >
            Moderation
          </GroupRow>
        )}

        {onManageChannel && (
          <GroupRow
            onClick={() => {
              onClose();
              onManageChannel();
            }}
            icon={<PencilSimpleLine size={ICON} className="text-textSecondary" />}
            trailing={<CaretRight size={16} className="text-textMuted" />}
          >
            Manage channel
          </GroupRow>
        )}

        {/* Only offered where the actions would actually succeed: Helix rejects
            them without mod powers in this specific channel. On or off reads
            from the color and the trailing word; the glyph keeps its weight. */}
        {isModerator && (
          <GroupRow
            onClick={onToggleModTools}
            icon={<ShieldCheck size={ICON} className={modToolsOn ? 'text-accent' : 'text-textSecondary'} />}
            value={modToolsOn ? 'On' : 'Off'}
          >
            Moderator tools
          </GroupRow>
        )}

        {/* The rows that take something away come last. */}
        {onCloseChat && (
          <GroupRow
            onClick={() => {
              onCloseChat();
              onClose();
            }}
            className="!text-error"
            icon={<X size={ICON} />}
          >
            Close this chat
          </GroupRow>
        )}

        {followRow && followSlot === 'bottom' && followRow}
      </GroupCard>
      {isModerator && modToolsOn && (
        <GroupFooter className="px-4 pt-2">Long-press any message for delete, timeout and ban.</GroupFooter>
      )}
    </MobileSheet>
  );
};
