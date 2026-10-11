// A pinned message's sender, or the moderator who pinned it, drawn the way a
// chat row draws them: Twitch badges in chat order, the 7TV badge, the other
// third-party badges, the StreamNook badge, then the name in its 7TV paint.
import { useMemo, type CSSProperties } from 'react';
import { parseBadges } from '../../services/twitchBadges';
import { useChatterLook } from '../../hooks/useChatterLook';
import { ChatBadgeIcons, type ResolvedTwitchBadge, type TwitchBadgeInfo } from './ChatBadgeIcons';
import type { ProviderId } from '../../types/providers';

const NO_BADGES: ResolvedTwitchBadge[] = [];

interface PinnedChatterProps {
  provider: ProviderId;
  userId: string;
  /** The platform display name. */
  name: string;
  /** The chatter's login, when the pin carries it (keys the chatter store). */
  login?: string;
  /** The name chat rows show for this chatter (`sender_shown_name`). */
  shownName?: string;
  color?: string;
  /** Twitch badges as the pin payload carries them, already in chat order. */
  badges?: ReadonlyArray<{ set_id: string; version: string }>;
  /** Broadcaster id, so subscriber and bits badges resolve to this channel's art. */
  channelId?: string;
  /**
   * `card`: the open pin; badges are clickable like a chat row's.
   * `line`: one line inside another control (the collapsed bar, a byline).
   * Badges are inert, and the ones that do not fit are hidden whole instead
   * of wrapping, so the line never grows a second row.
   */
  variant: 'card' | 'line';
  nameClassName?: string;
  /** Extra style for an unpainted name (a paint brings its own shadow). */
  plainNameStyle?: CSSProperties;
  /** Class for the badge strip, e.g. spacing or a width cap. */
  badgeClassName?: string;
  onBadgeClick?: (badgeKey: string, badgeInfo: TwitchBadgeInfo) => void;
}

export function PinnedChatter({
  provider,
  userId,
  name,
  login,
  shownName,
  color,
  badges,
  channelId,
  variant,
  nameClassName,
  plainNameStyle,
  badgeClassName = '',
  onBadgeClick,
}: PinnedChatterProps) {
  const badgeTag = badges?.map((b) => `${b.set_id}/${b.version}`).join(',') ?? '';
  const twitchBadges = useMemo<ResolvedTwitchBadge[]>(
    () => (badgeTag ? parseBadges(badgeTag, channelId) : NO_BADGES),
    [badgeTag, channelId],
  );
  const look = useChatterLook({ provider, userId, name, login, shownName, color, twitchBadges });
  const isLine = variant === 'line';

  return (
    <>
      {look.hasBadges && (
        <span
          className={`sn-badge-strip ${isLine ? 'min-w-0 shrink flex-wrap overflow-hidden' : ''} ${badgeClassName}`}
          // A line strip is exactly one badge tall: badges that do not fit wrap
          // into the clipped second row instead of being cut in half.
          style={isLine ? { height: 'calc(1.4em * var(--sn-badge-scale, 1))', verticalAlign: 'middle' } : undefined}
          data-no-pin-toggle={isLine ? undefined : ''}
        >
          <ChatBadgeIcons
            twitchBadges={look.twitchBadges}
            seventvBadge={look.seventvBadge}
            thirdPartyBadges={look.thirdPartyBadges}
            streamNookMemberId={look.streamNookMemberId}
            onBadgeClick={onBadgeClick}
            interactive={!isLine}
            keyPrefix="pin"
          />
        </span>
      )}
      <span
        className={nameClassName}
        style={look.painted ? look.nameStyle : { ...look.nameStyle, ...plainNameStyle }}
      >
        {look.displayName}
      </span>
    </>
  );
}
