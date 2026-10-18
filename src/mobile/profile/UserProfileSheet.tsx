// Tap a name in chat, see who they are, in the same shape as your own card:
// their Atmosphere glowing behind them, their picture with their StreamNook
// badge on it as a medal (never on your own), their name in their paint, their
// sub in this room, every badge they have under its platform's mark, and what
// they said this session. Under their sub, the viewer's own note and name
// color, and in a room the viewer moderates, Moderate and History
// (ChatterTools).
//
// Rust builds the sheet (`user_profile_sheet`, services/profile_view.rs): the
// badge groups merged and deduped per provider, the worn paint, the Helix
// picture and StreamNook membership, cached per person and room. The same
// answer draws the iPhone's native sheet, so the two cannot drift; this lays it
// out.
import React, { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import { listen } from '@tauri-apps/api/event';
import { historyKey } from '../../utils/chatterIdentity';
import type { ProfileBadge, ProfileSheetView } from '../../types/profileView';
import { FallbackImage } from '../../components/FallbackImage';
import { MobileSheet } from '../ui/MobileSheet';
import { useNameColorAdjust } from '../../hooks/useNameColor';
import { Logger } from '../../utils/logger';
import { SubStandingRow } from '../../components/profile/SubStandingRow';
import { wornSubBadgeArt, type WornBadge } from '../../utils/wornSubBadge';
import type { SubStanding } from '../../types';
import { AtmosphereGlow, BrandMark, CutoutAvatar, MemberRankMark, PaintedName, SectionHeader } from './IdentityParts';
import { badgeArt, brandFor, glowAtmosphere, identityNameSize, medalUrl } from './identity';
import { MemberSections } from './MemberSections';
import { ChatterTools } from './ChatterTools';

/** How far down the sheet their Atmosphere's glow reaches: the picture and
 *  the name, fading out above what follows. */
const GLOW = 200;
const AVATAR = 76;

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

const CLOCK = new Intl.DateTimeFormat([], { hour: 'numeric', minute: '2-digit' });

/** "7:59", the iPhone's stamp: the locale's own hours and minutes without the
 *  AM/PM, which wrapped the column onto a second line. */
const clock = (at: number | null): string =>
  at === null
    ? ''
    : CLOCK.formatToParts(at)
        .filter((p) => p.type !== 'dayPeriod')
        .map((p) => p.value)
        .join('')
        .trim();

/** Identifies one badge image for the failed-image set. The url is part of it
 *  because the cached sheet and the fresh one can name the same badge with
 *  different art. */
const badgeImageKey = (b: ProfileBadge): string => `${b.id}|${b.url}`;

/** The badges block while Rust has not answered yet: the heading and a row of
 *  tiles the size the badges land at, so the block arriving moves nothing. */
const BadgeGridPlaceholder: React.FC = () => (
  <div className="mt-5" aria-hidden="true">
    <SectionHeader title="Badges" className="mb-2" />
    <div className="flex gap-2">
      {[0, 1, 2].map((i) => (
        <span key={i} className="w-8 h-8 flex items-center justify-center">
          {/* glass-exempt: a placeholder in the theme's raised surface, solid
              so it reads the same at any Glassiness. */}
          <span
            className="block w-7 h-7 rounded-md"
            style={{ background: 'linear-gradient(var(--color-surface), var(--color-surface)), var(--color-background)' }}
          />
        </span>
      ))}
    </div>
  </div>
);

const BadgeGrid: React.FC<{
  title: string;
  /** Only the badges whose art has not failed; the caller drops a group once
   *  this is empty, so a heading never sits alone over nothing. */
  badges: ProfileBadge[];
  onPick: (b: ProfileBadge) => void;
  onImageFailed: (b: ProfileBadge) => void;
}> = ({ title, badges, onPick, onImageFailed }) => {
  if (!badges.length) return null;
  // A platform's group sits under its real mark; "Chat clients" keeps its words.
  const brand = brandFor(title);
  return (
    <div className="mt-5">
      <SectionHeader title={brand ? 'Badges' : title} brand={brand} detail={String(badges.length)} className="mb-2" />
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
              onAllFailed={() => onImageFailed(b)}
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
  // Retained so the sheet still has someone to show while it slides OUT.
  // `user` is already null by then, and returning nothing on null unmounted
  // the sheet outright, so it vanished instead of animating away.
  // Adjust-state-during-render, the same idiom SettingsScreen uses.
  const [lastUser, setLastUser] = useState(user);
  if (user && user !== lastUser) setLastUser(user);
  if (!lastUser) return null;
  // Keyed on the user, so opening a different profile MOUNTS a fresh one with
  // that user's cached cosmetics already seeded. Clearing state in an effect
  // instead would paint the previous person's badges under the new name for a
  // frame, and react-hooks rightly treats it as an error.
  return (
    <ProfileBody
      key={lastUser.userId}
      user={lastUser}
      open={!!user}
      channelId={channelId}
      channelName={channelName}
      onClose={onClose}
    />
  );
};

const ProfileBody: React.FC<{
  user: SheetUser;
  open: boolean;
  channelId?: string | null;
  channelName?: string | null;
  onClose: () => void;
}> = ({ user, open, channelId, channelName, onClose }) => {
  // Rust's answer: its cached copy first (a reopen paints at once), then fresh.
  const [sheet, setSheet] = useState<ProfileSheetView | null>(null);
  const [picked, setPicked] = useState<ProfileBadge | null>(null);
  // null until the store has answered, so "nothing yet" is never painted
  // over a list that is still on its way.
  const [history, setHistory] = useState<HistoryRow[] | null>(null);
  const [standing, setStanding] = useState<SubStanding | null>(null);
  // Badge art that failed every url it had. Those tiles are dropped, and a
  // group left with none is dropped with its heading.
  const [failedImages, setFailedImages] = useState<ReadonlySet<string>>(() => new Set());
  const markImageFailed = useCallback((b: ProfileBadge) => {
    const key = badgeImageKey(b);
    setFailedImages((prev) => (prev.has(key) ? prev : new Set(prev).add(key)));
  }, []);

  const userId = user.userId;
  const username = user.username;

  // What this person has said this session, from the Rust history store the
  // desktop profile card reads: the same rows a moderator looks at before
  // deciding, now reachable from a thumb. Seeded once, then kept live by the
  // store's push for as long as the sheet is open. The phone's chat tabs are
  // Twitch rooms, so the key is the bare id. The body outlives the sheet's
  // close (it stays up for the slide out), so the watch ends on close rather
  // than on unmount, and a reopen of the same person starts it again.
  useEffect(() => {
    if (!open) return;
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
  }, [userId, open]);

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

  const paint = sheet?.paint_def ?? null;
  const adjustNameColor = useNameColorAdjust();
  const shownColor = sheet?.name_color ?? adjustNameColor(user.color || '#9147FF') ?? '#9147FF';
  // A member's StreamNook group can carry art the app ships
  // (`streamnook-house:<slug>`), drawn from the bundle.
  const groups = (sheet?.groups ?? [])
    .map((group) => ({
      ...group,
      badges: group.badges
        .map((b) => ({ ...b, url: badgeArt(b.url) }))
        .filter((b) => !failedImages.has(badgeImageKey(b))),
    }))
    .filter((group) => group.badges.length > 0);
  // Your own picture carries no marks, here as on your card.
  const ownId = useAppStore((s) => s.currentUser?.user_id);
  const medal = !!sheet?.streamnook && !(ownId && ownId === (sheet?.user_id || user.userId));
  const name = sheet?.display_name || user.displayName;
  // Before Rust answers, the login line comes from what the row carried.
  const login =
    sheet?.shown_login ??
    (!sheet && user.username.toLowerCase() !== user.displayName.toLowerCase() ? user.username : null);
  // What they have on, in words: the Atmosphere, then the paint.
  const wearing = [sheet?.atmosphere?.name, paint?.name ? `${paint.name} paint` : null].filter(Boolean).join(' · ');
  const pickedBrand = brandFor(picked?.provider);

  return (
    <MobileSheet open={open} onClose={onClose} maxHeightFraction={0.8} fill>
      <div className="flex flex-col pb-2">
        {/* Their Atmosphere glowing behind the picture and name across the
            sheet's full width, else a faint glow of their name color. */}
        <div className="relative -mx-4 px-4 pt-3">
          <AtmosphereGlow atm={glowAtmosphere(sheet?.atmosphere?.id)} tint={shownColor} height={GLOW} />
          <div className="relative flex items-start gap-3">
            <CutoutAvatar
              platformUrl={sheet?.avatar_url}
              picture={sheet?.picture}
              size={AVATAR}
              medal={medal ? medalUrl(sheet?.streamnook) : null}
              medalLabel={sheet?.streamnook?.label}
            />
            <div className="flex-1" />
            {sheet?.member && (
              <div className="pt-1.5">
                <MemberRankMark rank={sheet.member} />
              </div>
            )}
          </div>
          <div className="relative pt-3 min-w-0">
            <PaintedName
              text={name}
              paint={paint}
              color={shownColor}
              className="block font-bold leading-tight break-words"
              style={{ fontSize: identityNameSize(name) - 2 }}
            />
            {(login || wearing) && (
              <div className="mt-0.5 flex items-center gap-1.5 min-w-0">
                {login && <span className="text-[13px] text-textMuted truncate shrink-0 max-w-[60%]">@{login}</span>}
                {login && wearing && <span className="text-[13px] text-textMuted">·</span>}
                {wearing && <span className="text-[12.5px] text-textSecondary truncate min-w-0">{wearing}</span>}
              </div>
            )}
          </div>
        </div>

        {standing && (
          <div className="mt-4">
            <SubStandingRow standing={standing} badgeArt={wornSubBadgeArt(wornBadges)} size="sheet" />
          </div>
        )}

        {/* The viewer's own note and name color for them, and Moderate and
            History where the viewer moderates (Rust decides who may be). */}
        {userId && (
          <ChatterTools
            userId={userId}
            login={username}
            displayName={user.displayName}
            channelId={channelId}
            channelName={channelName}
          />
        )}

        {/* Tapping a badge names it here rather than in a tooltip, which does
            nothing under a thumb. */}
        {picked && (
          <div className="mt-3 glass-panel p-3 flex items-start gap-2.5">
            <FallbackImage
              src={picked.url}
              fallbackUrls={picked.fallbacks}
              alt=""
              className="w-7 h-7 object-contain shrink-0"
            />
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-1.5 min-w-0">
                <span className="text-[14px] font-semibold text-textPrimary truncate">
                  {picked.title || picked.name}
                </span>
                {pickedBrand ? (
                  <BrandMark brand={pickedBrand} size={pickedBrand === 'seventv' ? 9 : 12} />
                ) : (
                  picked.provider && <span className="text-[12.5px] text-textMuted shrink-0">{picked.provider}</span>
                )}
              </div>
              {picked.description && (
                <div className="text-[12.5px] text-textSecondary mt-0.5">{picked.description}</div>
              )}
            </div>
          </div>
        )}

        {!sheet && <BadgeGridPlaceholder />}
        {groups.map((group) => (
          <BadgeGrid
            key={group.title}
            title={group.title}
            badges={group.badges}
            onPick={setPicked}
            onImageFailed={markImageFailed}
          />
        ))}

        {/* A member always has a StreamNook badge (it is the medal on their
            picture, and their StreamNook group), so the line is for others. */}
        {sheet && groups.length === 0 && !sheet.streamnook && (
          <div className="mt-4 text-[12.5px] text-textMuted">No badges to show for this user.</div>
        )}

        {sheet?.member_profile && <MemberSections profile={sheet.member_profile} />}

        {/* Chat order (oldest first, newest at the bottom), the way the eye
            already reads the room. Plain text: the point is what was said,
            and an emote's code says that as well as its picture. */}
        <div className="mt-5">
          <SectionHeader
            title="Recent messages"
            detail={history && history.length > 0 ? String(history.length) : null}
            className="mb-2"
          />
          {history === null ? null : history.length === 0 ? (
            <div className="text-[12.5px] text-textMuted">Nothing from them in this chat yet.</div>
          ) : (
            // One column for every row's time, as wide as the widest one
            // ("12:34 PM" on a 12-hour phone, "12:34" on a 24-hour one), so
            // a time never wraps and the messages start on one line.
            <ul className="grid grid-cols-[max-content_minmax(0,1fr)] gap-x-2.5 gap-y-1.5">
              {history.map((row) => (
                <li key={row.id} className="col-span-2 grid grid-cols-subgrid text-[14px] leading-snug">
                  <span className="whitespace-nowrap tabular-nums text-textMuted text-[12px] pt-[2px]">
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
