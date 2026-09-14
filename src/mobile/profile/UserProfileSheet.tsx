// Tap a name in chat, see who they are: every cosmetic source the app decorates
// a profile with, in one sheet.
//
// Replaces a 46-line stub that painted the display name and printed the 7TV
// badge as a line of TEXT. The data was always reachable — `useMobileBoot`
// already calls the same cache for the signed-in account — so this is mostly a
// matter of rendering what was already being fetched.
//
// Badge normalisation is shared with the desktop card via
// `utils/profileBadges`, because the merge and dedupe rules are per-source and
// fiddly: each provider spells its image fields differently, cache and Rust can
// both carry the same badge, and Twitch splits one badge across display and
// earned lists.
import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useChatUserStore } from '../../stores/chatUserStore';
import { computePaintStyle } from '../../services/seventvService';
import { getFullProfileWithFallback, getProfileFromMemoryCache } from '../../services/cosmeticsCache';
import { getStreamNookUserNumber } from '../../services/supabaseService';
import { normalizeProfileBadges, type NormalizedBadge } from '../../utils/profileBadges';
import { FallbackImage } from '../../components/FallbackImage';
import { StreamNookBadge } from '../../components/StreamNookBadge';
import { MobileSheet } from '../ui/MobileSheet';
import { Logger } from '../../utils/logger';

export interface SheetUser {
  userId: string;
  username: string;
  displayName: string;
  color: string;
}

const BadgeGrid: React.FC<{
  title: string;
  badges: NormalizedBadge[];
  onPick: (b: NormalizedBadge) => void;
}> = ({ title, badges: all, onPick }) => {
  // A source can hand back an entry with no usable image (a provider changing
  // its payload, mostly). Rendering it would just be a broken tile.
  const badges = all.filter((b) => !!b.src);
  if (!badges.length) return null;
  return (
    <div className="mt-3">
      <div className="text-[11px] font-semibold uppercase tracking-wide text-textMuted mb-1.5">
        {title}
      </div>
      <div className="flex flex-wrap gap-2">
        {badges.map((b, i) => (
          <button
            key={`${b.id}-${i}`}
            onClick={() => onPick(b)}
            className="w-8 h-8 flex items-center justify-center active:scale-90 transition-transform"
            aria-label={b.title || b.name || 'Badge'}
          >
            <FallbackImage
              src={b.src as string}
              fallbackUrls={b.fallbackUrls}
              srcSet={b.srcSet}
              alt={b.title || ''}
              className="w-7 h-7 object-contain"
            />
          </button>
        ))}
      </div>
    </div>
  );
};

export const UserProfileSheet: React.FC<{
  user: SheetUser | null;
  /** Channel context, so channel-scoped Twitch badges resolve. */
  channelId?: string | null;
  channelName?: string | null;
  onClose: () => void;
}> = ({ user, channelId, channelName, onClose }) => {
  if (!user) return null;
  // Keyed on the user, so opening a different profile MOUNTS a fresh one with
  // that user's cached cosmetics already seeded. Clearing state in an effect
  // instead would paint the previous person's badges under the new name for a
  // frame, and react-hooks rightly treats it as an error.
  return (
    <ProfileBody
      key={user.userId}
      user={user}
      channelId={channelId}
      channelName={channelName}
      onClose={onClose}
    />
  );
};

const ProfileBody: React.FC<{
  user: SheetUser;
  channelId?: string | null;
  channelName?: string | null;
  onClose: () => void;
}> = ({ user, channelId, channelName, onClose }) => {
  // The live chat store already holds paint and badge for anyone who has
  // spoken, so the sheet paints correctly on frame one and the fetch below only
  // fills in what chat does not carry.
  const storeUser = useChatUserStore((s) => s.users.get(user.userId));
  // Seeded synchronously from the LRU so reopening a profile does not flash.
  const [profile, setProfile] = useState(() => getProfileFromMemoryCache(user.userId));
  const [avatar, setAvatar] = useState<string | null>(null);
  const [picked, setPicked] = useState<NormalizedBadge | null>(null);

  const userId = user.userId;
  const username = user.username;

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const p = await getFullProfileWithFallback(
          userId,
          username,
          channelId ?? undefined,
          channelName ?? undefined,
        );
        if (!cancelled) setProfile(p);
      } catch (err) {
        Logger.debug('[UserProfile] cosmetics unavailable:', err);
      }
      try {
        const u = await invoke<{ profile_image_url?: string } | null>('get_user_by_id', {
          userId,
        });
        if (!cancelled && u?.profile_image_url) setAvatar(u.profile_image_url);
      } catch {
        /* avatar is decoration; a profile without one still renders */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [userId, username, channelId, channelName]);

  const paint = storeUser?.paint ?? profile?.seventvCosmetics?.paints?.find((p) => p.selected);
  const nameStyle = paint
    ? computePaintStyle(paint, user.color || '#9147FF', 'all')
    : { color: user.color || '#9147FF' };

  const badges = normalizeProfileBadges({ cachedProfile: profile });
  const userNumber = getStreamNookUserNumber(user.userId);

  return (
    <MobileSheet open={!!user} onClose={onClose} maxHeightFraction={0.8}>
      <div className="flex flex-col pb-2">
        <div className="flex items-center gap-3">
          {avatar ? (
            <img
              src={avatar}
              alt=""
              className="w-14 h-14 rounded-full object-cover shrink-0 ring-1 ring-white/10"
              draggable={false}
            />
          ) : (
            <div className="w-14 h-14 rounded-full bg-surface shrink-0" />
          )}
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-1.5 min-w-0">
              <span className="text-lg font-bold truncate" style={nameStyle}>
                {user.displayName}
              </span>
            </div>
            {user.displayName.toLowerCase() !== user.username.toLowerCase() && (
              <div className="text-[13px] text-textMuted truncate">@{user.username}</div>
            )}
            {paint?.name && (
              <div className="text-[11.5px] text-textSecondary mt-0.5 truncate">
                Paint: {paint.name}
              </div>
            )}
          </div>
        </div>

        {/* Tapping a badge names it here rather than in a tooltip, which does
            nothing under a thumb. */}
        {picked && (
          <div className="mt-3 px-2.5 py-1.5 rounded-lg bg-surface/60 text-[12px] text-textPrimary">
            <span className="font-semibold">{picked.title || picked.name}</span>
            {picked.provider && (
              <span className="text-textMuted"> · {picked.provider}</span>
            )}
            {picked.description && (
              <div className="text-[11.5px] text-textMuted mt-0.5">{picked.description}</div>
            )}
          </div>
        )}

        {/* Its OWN group, and gated on membership. StreamNookBadge carries no
            membership guard of its own — the desktop card gates the call site on
            `streamNookUserNumber !== null`, and its internal comment says as
            much ("shouldn't happen given isSN was true"). Rendered
            unconditionally it falls back to a plain "StreamNook Member" label,
            which hands every non-member a badge they do not have. */}
        {userNumber !== null && (
          <div className="mt-3">
            <div className="text-[11px] font-semibold uppercase tracking-wide text-textMuted mb-1.5">
              StreamNook
            </div>
            <StreamNookBadge userId={user.userId} userNumber={userNumber} side="bottom" />
          </div>
        )}

        <BadgeGrid title="Twitch" badges={badges.twitch} onPick={setPicked} />
        <BadgeGrid title="7TV" badges={badges.seventv} onPick={setPicked} />
        <BadgeGrid title="Chat clients" badges={badges.thirdParty} onPick={setPicked} />

        {badges.total === 0 && userNumber === null && (
          <div className="mt-4 text-[12.5px] text-textMuted text-center">
            No badges to show for this user.
          </div>
        )}
      </div>
    </MobileSheet>
  );
};
