// Your own identity, at the top of the You tab: avatar, 7TV-painted display
// name, and the badges you actually carry.
//
// Everything here is already in memory by the time this mounts. `useMobileBoot`
// pre-fetches the signed-in account's cosmetics at startup
// (registerOwnCosmeticAccounts -> revalidateOwnCosmetics -> getFullProfileWithFallback),
// so the LRU seed below paints on frame one and the fetch is only a refresh.
//
// Badge normalisation and paint computation are the SAME calls the chat row and
// the user profile sheet make, deliberately: a second implementation of "which
// badges does this person have" is how the two surfaces drift apart.
import React, { useEffect, useState } from 'react';
import { useChatUserStore } from '../../stores/chatUserStore';
import { computePaintStyle } from '../../services/seventvService';
import { getFullProfileWithFallback, getProfileFromMemoryCache } from '../../services/cosmeticsCache';
import { getStreamNookUserNumber } from '../../services/supabaseService';
import {
  getResolvedIdentity,
  getResolvedIdentityFromCache,
  subscribeResolvedIdentity,
  type ResolvedIdentity,
} from '../../services/identityService';
import { normalizeProfileBadges, type NormalizedBadge } from '../../utils/profileBadges';
import { FallbackImage } from '../../components/FallbackImage';
import { StreamNookBadge } from '../../components/StreamNookBadge';
import { Logger } from '../../utils/logger';

// The avatar stays with YouScreen: it is the one piece that needs no cosmetics
// lookup, and keeping it there lets the layout own its own alignment.
interface Props {
  userId: string;
  displayName: string;
  login?: string;
}

/**
 * Type size for the name.
 *
 * Twitch caps display names at 25 characters, so this is a bounded problem: the
 * longest possible name simply needs a smaller size to keep the header to two
 * lines. Stepping the size beats truncating, because the name is the one thing
 * on this screen the viewer came to see, and an ellipsis in your own name reads
 * like a bug.
 */
function nameSizeClass(name: string): string {
  const n = name.length;
  if (n > 20) return 'text-[14.5px]';
  if (n > 16) return 'text-[16px]';
  if (n > 12) return 'text-[17px]';
  return 'text-[18.5px]';
}

/** Twitch badge sets that only mean something inside one channel. */
const CHANNEL_SCOPED_SETS = new Set([
  'broadcaster',
  'moderator',
  'lead_moderator',
  'vip',
  'subscriber',
  'founder',
  'bits',
  'bits-leader',
  'sub-gifter',
  'sub-gift-leader',
  'artist-badge',
  'predictions',
  'hype-train',
  'clip-champ',
]);

export const OwnIdentityHeader: React.FC<Props> = ({ userId, displayName, login }) => {
  // Chat already holds paint and badge for anyone who has spoken, including
  // you, so prefer it and let the profile fill in what chat does not carry.
  const storeUser = useChatUserStore((s) => s.users.get(userId));
  const [profile, setProfile] = useState(() => getProfileFromMemoryCache(userId));

  // The StreamNook Identity loadout: the badges you chose to display on your
  // profile, resolved cross-client. This is the same set the desktop profile
  // card shows and the same set chat renders for a member, per chatUserStore:
  // "Members: their CURATED loadout ... which overrides their raw set."
  // useMobileBoot already warms it, so the cache seed usually paints at once.
  const [resolved, setResolved] = useState<ResolvedIdentity | null>(() =>
    getResolvedIdentityFromCache(userId),
  );
  useEffect(() => {
    let cancelled = false;
    void getResolvedIdentity(userId)
      .then((r) => {
        if (!cancelled) setResolved(r);
      })
      .catch(() => {
        /* not a member, or the identity API is unreachable */
      });
    // Equipping a badge elsewhere in the app writes through this cache, so the
    // header follows a change without needing a remount.
    const unsubscribe = subscribeResolvedIdentity((changedId) => {
      if (changedId === userId) setResolved(getResolvedIdentityFromCache(userId));
    });
    return () => {
      cancelled = true;
      unsubscribe();
    };
  }, [userId]);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        // Argument for argument what useMobileBoot already calls:
        // (selfId, selfLogin, selfId, selfLogin), with selfLogin falling back to
        // the username. Passing anything different here (a bare `login` that can
        // be undefined, say) keys a SECOND cache entry and throws away the warm
        // one the boot pre-fetch just filled.
        const selfLogin = login || displayName;
        const p = await getFullProfileWithFallback(userId, selfLogin, userId, selfLogin);
        if (!cancelled) setProfile(p);
      } catch (err) {
        Logger.debug('[OwnIdentity] cosmetics unavailable:', err);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [userId, login, displayName]);

  const paint = storeUser?.paint ?? profile?.seventvCosmetics?.paints?.find((p) => p.selected);
  // No paint is the common case and must stay legible: fall back to the primary
  // text colour rather than a chat username colour, because this is a heading
  // on a settings screen, not a chat line.
  const nameStyle = paint
    ? computePaintStyle(paint, 'var(--color-text-primary)', 'all')
    : undefined;

  const grouped = normalizeProfileBadges({ cachedProfile: profile });

  // Twitch: the badges you DISPLAY, never the earned inventory. Asked in the
  // context of your own channel, so channel-only badges (broadcaster, your own
  // sub or mod badge and the like) are dropped: the header is who you are
  // everywhere, not in one room.
  const displayed = new Set<string>(profile?.displayBadgeIds ?? []);
  const twitchWornIds = new Set<string>(
    ((profile?.twitchBadges ?? []) as { id?: string; setID?: string }[])
      .filter((b) => b.id && displayed.has(b.id) && !CHANNEL_SCOPED_SETS.has(b.setID ?? ''))
      .map((b) => b.id as string),
  );
  const twitchWorn = grouped.twitch.filter((b) => twitchWornIds.has(b.id));

  // WORN, not OWNED.
  //
  // `normalizeProfileBadges` returns COLLECTIONS, which is right for the profile
  // sheet (a scrollable inventory view) and wrong for a header. Measured on a
  // real account: 22 owned 7TV badges + 102 owned Twitch badges, and the first
  // cut of this header rendered all 127, burying the entire You tab under a wall
  // of icons. A header shows identity; the Cosmetics row directly below it is
  // where the collection belongs.
  //
  // 7TV: the same rule chatUserStore uses to pick the badge a chat row shows,
  // `badges.find((b) => b.selected)`, since 7TV marks only the active one.
  const selectedBadgeId: string | undefined =
    storeUser?.seventvBadge?.id ??
    profile?.seventvCosmetics?.badges?.find((b: { selected?: boolean; id?: string }) => b.selected)?.id;
  const seventvWorn = selectedBadgeId
    ? grouped.seventv.filter((b) => b.id === selectedBadgeId)
    : [];

  // Everything else comes from the Identity loadout, which is already the
  // curated "what I display" set rather than an inventory. It carries whatever
  // the member promoted into it, including a `twitch:` badge, so this covers
  // the other active badges without going near the raw Twitch group.
  //
  // Twitch's own group from the normaliser stays OUT: it merges
  // `display_badges` with `earned_badges`, so on this account it is 102 earned
  // badges rather than worn ones, and a settings screen has no channel context
  // to resolve worn ones from anyway.
  const loadoutBadges: NormalizedBadge[] = (resolved?.badges ?? []).map((b) => ({
    id: b.key,
    src: b.image_url,
    title: b.title,
    name: b.title,
    provider: b.provider,
  }));

  // Non-members have no loadout, so fall back to their raw third-party badges,
  // mirroring the member/non-member split in chatUserStore.
  const otherActive = loadoutBadges.length > 0 ? loadoutBadges : grouped.thirdParty;

  // Deduped because a 7TV badge promoted into the loadout would otherwise
  // appear twice. The slice is a backstop, not a design: nothing should reach
  // it, but this header must never again be able to eat the screen.
  const seen = new Set<string>();
  const badges: NormalizedBadge[] = [...twitchWorn, ...seventvWorn, ...otherActive]
    .filter((b) => {
      if (!b.src || seen.has(b.id)) return false;
      seen.add(b.id);
      return true;
    })
    .slice(0, 8);
  const userNumber = getStreamNookUserNumber(userId);

  return (
    <div className="min-w-0 flex-1">
      {/*
        Wrapping, not truncating, and deliberately NOT line-clamp: clamping
        needs `display: -webkit-box`, which fights the `background-clip: text`
        a 7TV paint is drawn with. `break-words` handles a long single-token
        name; the size step above keeps even a 25 character name to two lines.
      */}
      <div className="flex items-center gap-2 min-w-0">
        <div
          className={`font-bold text-textPrimary leading-tight break-words shrink-0 max-w-full ${nameSizeClass(displayName)}`}
          style={nameStyle}
        >
          {displayName}
        </div>
        {/* The applied 7TV paint by name, drawn in the paint itself, the same
            way the Cosmetics list previews one. It stays on the name's line and
            gives way first: a long paint name truncates, the name never does. */}
        {paint?.name && (
          <span className="glass-badge inline-flex items-center min-w-0 px-2 py-px rounded-full">
            <span
              className="text-[11px] font-bold truncate leading-snug"
              style={computePaintStyle(paint, '#9146FF')}
            >
              {paint.name}
            </span>
          </span>
        )}
      </div>

      {login && login.toLowerCase() !== displayName.toLowerCase() && (
        <div className="text-[13px] text-textMuted truncate">@{login}</div>
      )}

      {(badges.length > 0 || userNumber !== null) && (
        <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
          {userNumber !== null && (
            // Gated on membership: StreamNookBadge has no guard of its own and
            // falls back to a plain "StreamNook Member" label, which would hand
            // a badge to everyone who is not one.
            // `side="bottom"` because this sits at the top of the viewport and
            // a popover growing upward clips off screen.
            <StreamNookBadge userId={userId} userNumber={userNumber} side="bottom" />
          )}
          {badges.map((b, i) => (
            <FallbackImage
              key={`${b.id}-${i}`}
              src={b.src as string}
              fallbackUrls={b.fallbackUrls}
              srcSet={b.srcSet}
              alt={b.title || b.name || ''}
              title={b.title || b.name}
              className="w-[18px] h-[18px] object-contain shrink-0"
            />
          ))}
        </div>
      )}
    </div>
  );
};

export default OwnIdentityHeader;
