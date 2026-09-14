// The menu behind the composer's trailing button when there is nothing to send.
// Send and this share one slot, so the composer never carries a dead button.
import React from 'react';
import { ArrowsClockwise, ChatsCircle, Heart, HeartBreak, Lock, ShieldCheck, X } from 'phosphor-react';
import { MobileSheet } from '../ui/MobileSheet';
import { useChannelSocial } from '../../hooks/useChannelSocial';

interface Props {
  open: boolean;
  onClose: () => void;
  /** Null when no chat is open. */
  activeChannel: string | null;
  activeLabel: string | null;
  /** Broadcaster id, for follow/unfollow. Null when no chat is open. */
  channelId: string | null;
  isModerator: boolean;
  /** Active room restrictions, shown as context. Empty when unrestricted. */
  gatingLabels: string[];
  modToolsOn: boolean;
  onToggleModTools: () => void;
  onAddChat: () => void;
  onReload: () => void;
  /** Absent for the stream-following tab, which is not closable. */
  onCloseChat?: () => void;
}

export const ComposerMenuSheet: React.FC<Props> = ({
  open,
  onClose,
  activeChannel,
  activeLabel,
  channelId,
  isModerator,
  gatingLabels,
  modToolsOn,
  onToggleModTools,
  onAddChat,
  onReload,
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
  // The hook's SUBSCRIBE half is deliberately unused here: `handleSubscribeClick`
  // opens a webview window and is `#[cfg(desktop)]`, so there is no subscribe
  // control on mobile.
  const { isFollowing, followLoading, handleFollowClick } = useChannelSocial({
    userId: channelId,
    userLogin: activeChannel,
    userName: activeLabel,
    enabled: open && !!channelId,
  });

  return (
    <MobileSheet open={open} onClose={onClose} title={activeLabel ?? 'Chat'} maxHeightFraction={0.5}>
      <div className="flex flex-col">
        {/* Room state lives here rather than above the composer. Followers-only
            while you can still talk is context worth having, not a warning worth
            a permanent line over chat. */}
        {gatingLabels.length > 0 && (
          <div className="flex items-start gap-3 px-2 pb-2 mb-1 border-b border-borderSubtle">
            <Lock size={17} className="text-textSecondary shrink-0 mt-0.5" />
            <div className="flex-1 min-w-0">
              <div className="text-[13.5px] text-textSecondary">This chat is in</div>
              <div className="text-[14px] text-textPrimary">{gatingLabels.join(' · ')}</div>
            </div>
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
