// Tap a name in chat, see who they are: every cosmetic source the app decorates
// a profile with, in one sheet.
//
// Rust builds the sheet (`user_profile_sheet`, services/profile_view.rs): the
// badge groups merged and deduped per provider, the worn paint, the Helix
// picture and StreamNook membership, cached per person and room. The same
// answer draws the iPhone's native sheet, so the two cannot drift; this lays it
// out.
import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { historyKey } from '../../utils/chatterIdentity';
import { computePaintStyle } from '../../services/seventvService';
import type { ProfileBadge, ProfileSheetView } from '../../types/profileView';
import { FallbackImage } from '../../components/FallbackImage';
import { StreamNookBadge } from '../../components/StreamNookBadge';
import { MobileSheet } from '../ui/MobileSheet';
import { useNameColorAdjust } from '../../hooks/useNameColor';
import { Logger } from '../../utils/logger';
import { SevenTvAvatar } from '../../components/SevenTvAvatar';
import { SubStandingRow } from '../../components/profile/SubStandingRow';
import { wornSubBadgeArt, type WornBadge } from '../../utils/wornSubBadge';
import type { SubStanding } from '../../types';

export interface SheetUser {
  userId: string;
  username: string;
  displayName: string;
  color: string;
  /** The badges on the message the sheet was opened from, when there was one. */
  badges?: WornBadge[];
}

/** One row of what this person has said this session. */
interface HistoryRow {
  id: string;
  content: string;
  /** Unix ms, or null when the stamp could not be read. */
  at: number | null;
}

/** How many of a person's messages the sheet shows; the Rust store keeps more. */
const HISTORY_LIMIT = 50;

/** Rust stamps messages as unix milliseconds in a string; older rows and the
 *  push path can carry an RFC 3339 stamp instead. Read either. */
const toMillis = (ts: string | undefined): number | null => {
  if (!ts) return null;
  const n = Number(ts);
  if (Number.isFinite(n)) return n < 1e12 ? n * 1000 : n;
  const parsed = Date.parse(ts);
  return Number.isFinite(parsed) ? parsed : null;
};

const clock = (at: number | null): string =>
  at === null ? '' : new Date(at).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });

const BadgeGrid: React.FC<{
  title: string;
  badges: ProfileBadge[];
  onPick: (b: ProfileBadge) => void;
}> = ({ title, badges, onPick }) => {
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
              src={b.url}
              fallbackUrls={b.fallbacks}
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
  // Rust's answer: its cached copy first (a reopen paints at once), then fresh.
  const [sheet, setSheet] = useState<ProfileSheetView | null>(null);
  const [picked, setPicked] = useState<ProfileBadge | null>(null);
  // null until the store has answered, so "nothing yet" is never painted
  // over a list that is still on its way.
  const [history, setHistory] = useState<HistoryRow[] | null>(null);
  const [standing, setStanding] = useState<SubStanding | null>(null);

  const userId = user.userId;
  const username = user.username;

  // What this person has said this session, from the Rust history store the
  // desktop profile card reads: the same rows a moderator looks at before
  // deciding, now reachable from a thumb. Seeded once, then kept live by the
  // store's push for as long as the sheet is open. The phone's chat tabs are
  // Twitch rooms, so the key is the bare id.
  useEffect(() => {
    let cancelled = false;
    const key = historyKey(userId, 'twitch');
    void invoke<{ id: string; content: string; timestamp?: string }[]>(
      'get_user_message_history_limited',
      { userId: key, limit: HISTORY_LIMIT },
    )
      .then((rows) => {
        if (cancelled) return;
        setHistory(
          (rows ?? []).map((r) => ({ id: r.id, content: r.content, at: toMillis(r.timestamp) })),
        );
      })
      .catch(() => {
        if (!cancelled) setHistory([]);
      });
    void invoke('watch_user_history', { userKey: key }).catch(() => {});
    const unlisten = listen<{
      user_key: string;
      message: { id?: string; content?: string; timestamp?: string };
    }>('user-history-message', (event) => {
      if (cancelled || event.payload.user_key !== key) return;
      const m = event.payload.message;
      const id = m?.id;
      if (!id) return;
      setHistory((prev) => {
        const rows = prev ?? [];
        if (rows.some((r) => r.id === id)) return prev;
        const row: HistoryRow = { id, content: m.content ?? '', at: toMillis(m.timestamp) };
        const next = rows.concat(row);
        return next.length > HISTORY_LIMIT ? next.slice(next.length - HISTORY_LIMIT) : next;
      });
    });
    return () => {
      cancelled = true;
      unlisten.then((fn) => fn());
      void invoke('unwatch_user_history', { userKey: key }).catch(() => {});
    };
  }, [userId]);

  // Their sub in this room, with Rust's words for it. The sheet is keyed on the
  // user, so the badges it was opened with are fixed for its life.
  const wornBadges = user.badges;
  useEffect(() => {
    if (!channelName) return;
    let cancelled = false;
    void invoke<SubStanding | null>('get_sub_standing', {
      login: username,
      channel: channelName,
      wornBadges: (wornBadges ?? []).map((b) => b.key),
    })
      .then((s) => {
        if (!cancelled) setStanding(s);
      })
      .catch((err) => Logger.debug('[UserProfile] sub standing unavailable:', err));
    return () => {
      cancelled = true;
    };
  }, [username, channelName, wornBadges]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const cached = await invoke<ProfileSheetView | null>('user_profile_sheet_cached', {
          userId,
          channelId: channelId ?? null,
        });
        if (!cancelled && cached) setSheet(cached);
        const fresh = await invoke<ProfileSheetView>('user_profile_sheet', {
          userId,
          login: username,
          displayName: user.displayName,
          color: user.color || null,
          channelId: channelId ?? null,
          channelLogin: channelName ?? null,
        });
        if (!cancelled) setSheet(fresh);
      } catch (err) {
        Logger.debug('[UserProfile] profile unavailable:', err);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [userId, username, user.displayName, user.color, channelId, channelName]);

  const paint = sheet?.paint_def;
  const adjustNameColor = useNameColorAdjust();
  const shownColor = sheet?.name_color ?? adjustNameColor(user.color || '#9147FF') ?? '#9147FF';
  const nameStyle = paint ? computePaintStyle(paint, shownColor, 'all') : { color: shownColor };
  const avatar = sheet?.avatar_url ?? null;
  const groups = sheet?.groups ?? [];
  const member = !!sheet?.streamnook;

  return (
    <MobileSheet open={!!user} onClose={onClose} maxHeightFraction={0.8}>
      <div className="flex flex-col pb-2">
        <div className="flex items-center gap-3">
          {avatar ? (
            <SevenTvAvatar
              cosmeticsId={userId}
              px={56}
              motion="animate"
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

        {standing && (
          <div className="mt-3">
            <SubStandingRow standing={standing} badgeArt={wornSubBadgeArt(wornBadges)} size="sheet" />
          </div>
        )}

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

        {groups.map((group) => (
          <BadgeGrid key={group.title} title={group.title} badges={group.badges} onPick={setPicked} />
        ))}

        {/* Last, as on the desktop card and in chat (utils/badgeOrder). Its OWN
            group, and only for a member (Rust sends `streamnook` for members
            alone): StreamNookBadge carries no membership guard of its own and
            would hand every non-member a plain "StreamNook" badge. */}
        {member && (
          <div className="mt-3">
            <div className="text-[11px] font-semibold uppercase tracking-wide text-textMuted mb-1.5">
              StreamNook
            </div>
            <StreamNookBadge userId={user.userId} side="bottom" />
          </div>
        )}

        {sheet && groups.length === 0 && !member && (
          <div className="mt-4 text-[12.5px] text-textMuted text-center">
            No badges to show for this user.
          </div>
        )}

        {/* Chat order (oldest first, newest at the bottom), the way the eye
            already reads the room. Plain text: the point is what was said,
            and an emote's code says that as well as its picture. */}
        <div className="mt-4">
          <div className="text-[11px] font-semibold uppercase tracking-wide text-textMuted mb-1.5">
            Recent messages
            {history && history.length > 0 && (
              <span className="ml-1.5 font-medium normal-case tracking-normal">{history.length}</span>
            )}
          </div>
          {history === null ? null : history.length === 0 ? (
            <div className="text-[12.5px] text-textMuted">
              Nothing from them in this chat yet.
            </div>
          ) : (
            <ul className="flex flex-col gap-1">
              {history.map((row) => (
                <li key={row.id} className="flex gap-2 text-[13px] leading-snug">
                  <span className="shrink-0 tabular-nums text-textMuted text-[11.5px] pt-[1px] w-[4.5ch]">
                    {clock(row.at)}
                  </span>
                  <span className="min-w-0 break-words text-textPrimary">{row.content}</span>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>
    </MobileSheet>
  );
};
