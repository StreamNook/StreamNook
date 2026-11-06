import { useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { motion } from 'framer-motion';
import {
  Clock,
  Cake,
  Users,
  MessageSquare,
  Tv,
  Gift,
  HandHeart,
  Coins,
  Palette,
  Award,
  Dices,
  Sparkles,
  Trophy,
  Lock,
  Heart,
  Star,
  type LucideIcon,
} from 'lucide-react';
import { Tooltip } from '../ui/Tooltip';
import { TwitchGlyph, TwitchVerifiedMark } from '../ui/TwitchGlyph';
import { Logger } from '../../utils/logger';
import { formatShortCount } from '../../utils/streamStats';
import { fitHeaderDensity } from '../chat/fitHeaderDensity';
import { ACCOLADE_ICONS } from '../profile/accoladeIcons';
import { buildAccoladeWall, type AccoladeWall, type WallItem } from './accoladeWall';
import { useOwnAccolades } from '../../hooks/useOwnAccolades';
import {
  getUserStats,
  getAccolades,
  getFavoriteChannel,
  type UserStats,
  type ChannelWatch,
} from '../../services/supabaseService';
import { getTier } from '../StreamNookBadge';
import type { InventoryResponse, ChannelPointsBalance, IvrUserSummary } from '../../types';
import { pickHoursRoast, type PickedRoast } from '../../utils/hoursWatchedRoasts';
import SubscriptionsSection from './SubscriptionsSection';
import MembershipSection from './MembershipSection';
import TopEmotesSection from './TopEmotesSection';
import { ProfileAccentContext, ProfileCompactContext } from './profileAccentContext';
import { isStatHidden, type ProfileStatKey } from '../../services/profileVisibility';

interface ProfileOverviewProps {
  userId: string;
  login: string;
  // 'partner' | 'affiliate' | '' (plain viewer). Fallback only; IVR roles win.
  broadcasterType: string;
  streamNookUserNumber: number | null;
  seventvPaintCount: number;
  seventvBadgeCount: number;
  ownedCosmeticsCount: number;
  // When false, render another member's PUBLIC profile: skip the self-only
  // live data (drops, channel-points balances, subscriptions) and draw the
  // public wall. Defaults to true (the current settings usage).
  isOwnProfile?: boolean;
  // Section keys the member hid from their public profile (honored only when
  // !isOwnProfile). Keys: roast, twitch, lifetime, emotes, accolades.
  hiddenSections?: string[];
  // The viewed member's display name, used for contextual possessive labels
  // (e.g. "Tyler's Twitch") when viewing someone else. Falls back to `login`.
  displayName?: string;
}

// Per-stat icon colors. The color now lives on the ICON itself (vibrant, but
// not neon), not on a flat background chip — so the row reads as a clean set of
// colored marks rather than a rainbow of pressable-looking tiles.
const ICON_COLOR = {
  violet: '#b9a6ec',
  sky: '#8fc3e8',
  green: '#8fd4b4',
  amber: '#e6c178',
  rose: '#e493a0',
  slate: '#aebfca',
};


// Stagger so tiles slide in one after another instead of popping as a block.
const containerV = {
  hidden: {},
  show: { transition: { staggerChildren: 0.045 } },
};
const itemV = {
  hidden: { opacity: 0, y: 8 },
  show: { opacity: 1, y: 0, transition: { duration: 0.25, ease: 'easeOut' as const } },
};

// rAF count-up so numbers tick from zero to their value on first paint (and
// re-animate when the real value arrives after the async fetch resolves).
const useCountUp = (target: number, durationMs = 900): number => {
  const [value, setValue] = useState(0);
  useEffect(() => {
    if (!target || target <= 0) return;
    let raf = 0;
    let start = 0;
    const tick = (ts: number) => {
      if (!start) start = ts;
      const p = Math.min(1, (ts - start) / durationMs);
      const eased = 1 - Math.pow(1 - p, 3); // easeOutCubic
      setValue(Math.round(target * eased));
      if (p < 1) raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [target, durationMs]);
  // A non-positive target has nothing to count towards; derive the rest state
  // instead of writing it back through the effect.
  return target > 0 ? value : 0;
};

const StatTile = ({
  icon: Icon,
  glyph,
  label,
  count,
  value,
  caption,
  color,
  tooltip,
}: {
  icon?: LucideIcon;
  // A branded mark (e.g. the Twitch logo) shown instead of a Lucide icon. The
  // caller controls its size/color.
  glyph?: ReactNode;
  label: string;
  // Provide `count` for an animated number, or `value` for a fixed string.
  count?: number;
  value?: string;
  caption?: string;
  // Vibrant color applied to the ICON itself (no background chip).
  color?: string;
  tooltip?: string;
}) => {
  const animated = useCountUp(count ?? 0);
  const display = value !== undefined ? value : animated.toLocaleString();
  const accentRgb = useContext(ProfileAccentContext);
  const compact = useContext(ProfileCompactContext);
  // A count too wide for its tile switches to its short form ("123.5M")
  // instead of being cut with an ellipsis (globals.css `.sn-stat-value`).
  const [valueEl, setValueEl] = useState<HTMLDivElement | null>(null);
  useEffect(() => (valueEl ? fitHeaderDensity(valueEl, 1, 0) : undefined), [valueEl]);
  const inner = (
    // glass-tile = display depth (top-lit material + layering), NOT a button.
    <div
      className={`glass-tile h-full ${compact ? 'p-2.5' : 'p-4'}`}
      style={accentRgb ? { borderColor: `rgba(${accentRgb}, 0.16)` } : undefined}
    >
      <div className={`flex items-center gap-1.5 ${compact ? 'mb-1' : 'mb-2'}`}>
        <span className="flex flex-shrink-0 items-center justify-center" style={color ? { color } : undefined}>
          {glyph ?? (Icon ? <Icon size={compact ? 15 : 17} strokeWidth={2.25} /> : null)}
        </span>
        <span className={`font-medium uppercase tracking-[0.14em] text-textMuted ${compact ? 'text-[10px]' : 'text-[11px]'}`}>
          {label}
        </span>
      </div>
      {value !== undefined ? (
        <div className={`truncate font-bold tabular-nums text-textPrimary ${compact ? 'text-base' : 'text-2xl'}`}>{display}</div>
      ) : (
        <div
          ref={setValueEl}
          className={`sn-stat-value truncate font-bold tabular-nums text-textPrimary ${compact ? 'text-base' : 'text-2xl'}`}
        >
          <span className="sn-stat-full">{display}</span>
          <span className="sn-stat-short">{formatShortCount(animated)}</span>
        </div>
      )}
      {caption && <div className={`leading-snug text-textSecondary ${compact ? 'mt-0.5 text-[10px]' : 'mt-1 text-[11px]'}`}>{caption}</div>}
    </div>
  );
  return (
    <motion.div variants={itemV} className="h-full min-w-0">
      {tooltip ? (
        <Tooltip content={tooltip} side="top">
          {inner}
        </Tooltip>
      ) : (
        inner
      )}
    </motion.div>
  );
};

// Bevel comes from inset shadows only, no outer glow, per the house style.
const MEDALLION_BEVEL =
  'inset 1.5px 1.5px 1px rgba(255,255,255,0.35), inset -1.5px -1.5px 3px rgba(0,0,0,0.30)';

const AccoladeMedallion = ({ a }: { a: WallItem }) => {
  const Icon = ACCOLADE_ICONS[a.icon] ?? Star;
  // Vivid two-stop gradient fill for the earned medallion. Each accolade gets
  // its own color so the wall reads as a varied collection.
  const grad = `linear-gradient(140deg, ${a.colors[0]}, ${a.colors[1]})`;
  const compact = useContext(ProfileCompactContext);
  const mystery = a.secret && !a.earned;
  const label = mystery ? '???' : a.label;
  // The wall has already chosen the words for this viewer. On YOUR OWN profile
  // a secret accolade shows its cryptic, guiding hint so you can discover and
  // chase it. On someone else's it never reveals the how-to (so it can't be
  // cheated by reading it off their earned one), and a stat they hid never
  // shows its number.
  return (
    <Tooltip content={a.hint} side="top">
      <motion.div
        variants={itemV}
        className={`flex flex-col items-center text-center ${compact ? 'gap-1.5' : 'gap-2'}`}
      >
        <div
          className={`relative flex ${compact ? 'h-12 w-12' : 'h-16 w-16'} items-center justify-center rounded-full ${
            a.earned ? '' : 'border border-white/[0.06] bg-white/[0.03] opacity-55'
          }`}
          style={a.earned ? { backgroundImage: grad, boxShadow: MEDALLION_BEVEL } : undefined}
        >
          {a.earned ? (
            <Icon
              size={compact ? 20 : 26}
              strokeWidth={2}
              className="text-white drop-shadow-[0_1px_1px_rgba(0,0,0,0.35)]"
            />
          ) : mystery ? (
            <span className="text-lg font-bold text-textMuted">?</span>
          ) : (
            <Lock size={compact ? 18 : 20} strokeWidth={2.25} className="text-textMuted" />
          )}
        </div>
        <span
          className={`font-medium leading-tight ${compact ? 'max-w-[68px] text-[10px]' : 'max-w-[80px] text-[11px]'} ${
            a.earned ? 'text-textPrimary' : 'text-textMuted'
          }`}
        >
          {label}
        </span>
      </motion.div>
    </Tooltip>
  );
};

// "2 yrs, 4 mos" age, a numeric year count (for milestones), the exact join
// date, and a days-until-next-cake-day caption.
const describeAccountAge = (createdAt: string | null) => {
  if (!createdAt) return null;
  const created = new Date(createdAt);
  if (Number.isNaN(created.getTime())) return null;
  const now = new Date();

  let years = now.getFullYear() - created.getFullYear();
  let months = now.getMonth() - created.getMonth();
  if (now.getDate() < created.getDate()) months -= 1;
  if (months < 0) {
    years -= 1;
    months += 12;
  }

  let next = new Date(now.getFullYear(), created.getMonth(), created.getDate());
  if (next.getTime() < now.getTime()) {
    next = new Date(now.getFullYear() + 1, created.getMonth(), created.getDate());
  }
  const daysToCakeDay = Math.ceil((next.getTime() - now.getTime()) / (1000 * 60 * 60 * 24));

  const ageLabel =
    years > 0
      ? `${years} yr${years === 1 ? '' : 's'}${months > 0 ? `, ${months} mo${months === 1 ? '' : 's'}` : ''}`
      : `${months} mo${months === 1 ? '' : 's'}`;

  const exact = created.toLocaleDateString('en-US', {
    year: 'numeric',
    month: 'long',
    day: 'numeric',
  });

  const caption =
    daysToCakeDay <= 0
      ? 'Cake day is today. Make a wish.'
      : daysToCakeDay === 1
        ? 'Cake day tomorrow'
        : `Cake day in ${daysToCakeDay} days`;

  return { ageLabel, years, caption, exact };
};

// "Tyler" -> "Tyler's", "Chris" -> "Chris'". Used for contextual section labels
// when viewing another member's profile.
const possessive = (name: string): string =>
  /s$/i.test(name) ? `${name}'` : `${name}'s`;

const ProfileOverview = ({
  userId,
  login,
  broadcasterType,
  streamNookUserNumber,
  seventvPaintCount,
  seventvBadgeCount,
  ownedCosmeticsCount,
  isOwnProfile = true,
  hiddenSections = [],
  displayName,
}: ProfileOverviewProps) => {
  const [stats, setStats] = useState<UserStats | null>(null);
  const [followers, setFollowers] = useState<number | null>(null);
  const [createdAt, setCreatedAt] = useState<string | null>(null);
  const [roles, setRoles] = useState<{ isAffiliate: boolean; isPartner: boolean; isStaff: boolean } | null>(null);
  const [dropsClaimed, setDropsClaimed] = useState<number | null>(null);
  const [earnedAccolades, setEarnedAccolades] = useState<Set<string>>(new Set());
  // Real channel-points holdings: the summed current balance across every
  // channel StreamNook has tracked for you (not the auto-collected counter).
  const [pointsHeld, setPointsHeld] = useState<number | null>(null);
  const [pointsChannels, setPointsChannels] = useState<number>(0);
  const [topPointsChannel, setTopPointsChannel] = useState<string | null>(null);
  const [favoriteChannel, setFavoriteChannel] = useState<ChannelWatch | null>(null);
  const [roast, setRoast] = useState<PickedRoast | null>(null);
  const [showAllAccolades, setShowAllAccolades] = useState(false);

  // Twitch Recap from before joining, as the server counted it, simply counts:
  // it rides along with the tracked stats everywhere they are used (accolades
  // too), with no mark of where it came from.
  const recapHours = Number(stats?.recap_hours ?? 0);
  const hours = (stats?.hours_watched ?? 0) + recapHours;
  const animatedHours = useCountUp(Math.round(hours));

  // The line shown last, so a re-roll never repeats it. It lives as long as this
  // profile is open; reopening starts fresh.
  const lastRoast = useRef<string | null>(null);
  const rerollRoast = () => {
    const picked = pickHoursRoast(hours, Math.random, lastRoast.current);
    lastRoast.current = picked.text;
    setRoast(picked);
  };
  useEffect(() => {
    if (!stats) return;
    const picked = pickHoursRoast((stats.hours_watched ?? 0) + Number(stats.recap_hours ?? 0), Math.random, lastRoast.current);
    lastRoast.current = picked.text;
    setRoast(picked);
  }, [stats]);

  useEffect(() => {
    let alive = true;

    getUserStats(userId)
      .then((s) => {
        if (alive) setStats(s ?? ({ user_id: userId, channel_points_collected: 0, hours_watched: 0, messages_sent: 0, streams_watched: 0, updated_at: '' } as UserStats));
      })
      .catch((e) => Logger.error('[ProfileOverview] stats:', e));

    getFavoriteChannel(userId)
      .then((c) => { if (alive) setFavoriteChannel(c); })
      .catch(() => {});

    getAccolades(userId)
      .then((ids) => {
        if (!alive) return;
        setEarnedAccolades(new Set(ids));
      })
      .catch(() => {});

    invoke<IvrUserSummary | null>('get_ivr_user_summary', { login })
      .then((u) => {
        if (!alive || !u) return;
        setFollowers(u.followers);
        setCreatedAt(u.created_at);
        setRoles({
          isAffiliate: u.is_affiliate,
          isPartner: u.is_partner,
          isStaff: u.is_staff,
        });
      })
      .catch((e) => Logger.error('[ProfileOverview] ivr:', e));

    // Live, self-only data tied to the viewer's own session (drops + channel
    // points balances). Skipped when viewing another member's public profile.
    if (isOwnProfile) {
      invoke<InventoryResponse>('get_drops_inventory')
        .then((inv) => {
          if (!alive) return;
          // Drops claimed = total awards across the account's permanent
          // earned-drops inventory (a drop can be awarded more than once), the
          // same count the Drops center shows. completed_drops.length undercounts
          // it by collapsing repeats into one.
          const completed = inv?.completed_drops ?? [];
          setDropsClaimed(completed.reduce((sum, d) => sum + (d.total_count || 1), 0));
        })
        .catch(() => { if (alive) setDropsClaimed(null); });

      const applyBalances = (balances: ChannelPointsBalance[]) => {
        if (!alive) return;
        const list = Array.isArray(balances) ? balances : [];
        setPointsHeld(list.reduce((sum, b) => sum + (b.balance || 0), 0));
        setPointsChannels(list.length);
        // Most-points channel ~ where you've banked the most channel points.
        const top = list.reduce<ChannelPointsBalance | null>(
          (best, b) => (b.balance > (best?.balance ?? -1) ? b : best),
          null,
        );
        setTopPointsChannel(top?.channel_name ?? null);
      };
      // Pull every followed channel's balance so held-points (and the Point
      // Hoarder/Tycoon accolades) reflect the whole account, not just channels
      // watched this session. Falls back to the warm store if the walk fails.
      invoke<ChannelPointsBalance[]>('refresh_followed_channel_points')
        .then(applyBalances)
        .catch(() => {
          invoke<ChannelPointsBalance[]>('get_all_channel_points_balances')
            .then(applyBalances)
            .catch(() => { if (alive) setPointsHeld(null); });
        });
    }

    return () => { alive = false; };
  }, [userId, login]);

  const age = useMemo(() => describeAccountAge(createdAt), [createdAt]);
  const tier = streamNookUserNumber !== null ? getTier(streamNookUserNumber) : null;

  // Account type: trust IVR roles (reliable) over the often-empty
  // broadcaster_type on the user object.
  const accountType = roles?.isStaff
    ? 'Twitch Staff'
    : roles?.isPartner || broadcasterType === 'partner'
      ? 'Partner'
      : roles?.isAffiliate || broadcasterType === 'affiliate'
        ? 'Affiliate'
        : 'Viewer';

  const cosmeticsTotal = seventvPaintCount + seventvBadgeCount + ownedCosmeticsCount;
  const messages = (stats?.messages_sent ?? 0) + Number(stats?.recap_messages ?? 0);
  const subsGifted = Number(stats?.recap_subs_gifted ?? 0);
  const recapPoints = Number(stats?.recap_points ?? 0);
  const streams = stats?.streams_watched ?? 0;

  const years = age?.years ?? 0;

  // The wall: which accolades this member can still reach, which they hold, and
  // the words each carries for the person looking. YOUR wall is Rust's answer:
  // it counts what every platform counts, saves what you earn (nothing here
  // saves an accolade), shows a locked secret as "???" with its hint, and prints
  // your numbers in the hints. Someone else's is built here from what they show
  // the public: a stat they hid (or one the public view never reads) never shows
  // its number, and locked secrets stay off the wall.
  const ownWall = useOwnAccolades(isOwnProfile ? userId : null);
  const wall: AccoladeWall | null = isOwnProfile
    ? ownWall.wall
    : buildAccoladeWall({
        messages,
        hours,
        streams,
        drops: 0,
        points: 0,
        cosmetics: cosmeticsTotal,
        years,
        member_number: streamNookUserNumber,
        saved: [...earnedAccolades],
        hidden: hiddenSections,
      });
  const sortedAccolades = wall?.items ?? [];
  const earnedMedallions = sortedAccolades.filter((a) => a.earned);
  const lockedCount = wall ? wall.total - wall.earned : 0;

  const accentRgb = useContext(ProfileAccentContext);
  const compact = useContext(ProfileCompactContext);
  // A member can hide stats from their PUBLIC profile, one by one; honored only
  // when viewing someone else (the self settings view always shows everything).
  const statHidden = (key: ProfileStatKey) => !isOwnProfile && isStatHidden(hiddenSections, key);
  const twitchShown = !(statHidden('twitch_age') && statHidden('followers') && statHidden('account_type'));
  const lifetimeShown =
    isOwnProfile ||
    !(
      statHidden('messages') &&
      statHidden('streams') &&
      statHidden('cosmetics') &&
      statHidden('favorite_channel') &&
      statHidden('subs_gifted') &&
      statHidden('member_rank')
    );
  const sectionStyle = accentRgb ? { borderColor: `rgba(${accentRgb}, 0.3)` } : undefined;
  const sectionPad = compact ? 'p-3.5' : 'p-4';
  const gridGap = compact ? 'gap-2' : 'gap-3';
  const headMb = compact ? 'mb-2.5' : 'mb-4';
  // Contextual section label: your own profile says "Your", another member's
  // says e.g. "Tyler's".
  const twitchLabel = isOwnProfile ? 'Your Twitch' : `${possessive(displayName ?? login)} Twitch`;
  // In the compact overlay, show only earned accolades by default (it's a public
  // showcase) with an expander for the locked ones, so the wall doesn't dominate.
  const accoladesShown = compact && !showAllAccolades ? earnedMedallions : sortedAccolades;

  // Each section is computed once, then arranged differently per layout:
  // compact (public/preview) is a wide landscape grid; full (Settings) is the
  // original vertical stack. Sections keep their own hide guards.

  // Hours-watched roast hero. The whole card is a re-roll button (pressable).
  const heroEl = statHidden('hours') ? null : (
    <motion.button
      type="button"
      onClick={compact ? undefined : rerollRoast}
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.3, ease: 'easeOut' }}
      className={`w-full rounded-xl glass-panel text-left transition-colors ${
        compact ? 'h-full cursor-default p-4' : 'group p-6 hover:bg-white/[0.02]'
      }`}
      style={sectionStyle}
    >
      <div className={`flex items-center gap-2 ${compact ? 'mb-2' : 'mb-3'}`}>
        <Clock size={16} strokeWidth={2.25} style={{ color: ICON_COLOR.amber }} />
        <span className="text-[11px] font-medium uppercase tracking-[0.14em] text-textMuted">
          Hours watched
        </span>
      </div>
      <div className="flex items-baseline gap-2">
        <span className={`font-bold tabular-nums text-textPrimary ${compact ? 'text-4xl' : 'text-5xl'}`}>
          {animatedHours.toLocaleString()}
        </span>
        <span className="text-sm text-textMuted">hrs</span>
      </div>
      {roast && !statHidden('roast_line') && (
        <div className={`flex items-start justify-between gap-3 ${compact ? 'mt-2' : 'mt-3'}`}>
          <p className={`leading-relaxed text-textSecondary ${compact ? 'text-[12px]' : 'text-[15px]'}`}>
            {roast.text}
          </p>
          {!compact && (
            <span className="mt-0.5 inline-flex flex-shrink-0 items-center gap-1.5 whitespace-nowrap text-[11px] font-medium text-textMuted opacity-0 transition-opacity group-hover:opacity-100">
              <Dices size={12} /> Tap for another
            </span>
          )}
        </div>
      )}
    </motion.button>
  );

  const twitchEl = !twitchShown ? null : (
    <div className={`settings-card ${sectionPad}`} style={sectionStyle}>
      <h4 className={`${headMb} flex items-center gap-1.5 text-[12px] font-semibold uppercase tracking-[0.14em] text-textPrimary`}>
        <TwitchGlyph size={14} className="text-[#9146FF]" /> {twitchLabel}
      </h4>
      <motion.div
        variants={containerV}
        initial="hidden"
        animate="show"
        className={`grid grid-cols-2 ${gridGap} ${compact ? '' : 'sm:grid-cols-3'}`}
      >
        {statHidden('twitch_age') ? null : age ? (
          <StatTile icon={Cake} label="Twitch age" value={age.ageLabel} caption={age.caption} color={ICON_COLOR.rose} tooltip={`Joined ${age.exact}`} />
        ) : (
          <StatTile icon={Cake} label="Twitch age" value="Unknown" color={ICON_COLOR.rose} />
        )}
        {!statHidden('followers') && (
          <StatTile
            icon={Users}
            label="Followers"
            count={followers ?? undefined}
            value={followers === null ? 'Unknown' : undefined}
            color={ICON_COLOR.sky}
          />
        )}
        {/* Account type spans the row in the compact 2-col grid so it fills. */}
        {!statHidden('account_type') && (
        <div className={compact ? 'col-span-2' : undefined}>
          <StatTile
            glyph={
              accountType === 'Partner' || accountType === 'Twitch Staff' ? (
                <TwitchVerifiedMark size={17} className="text-[#9146FF]" />
              ) : accountType === 'Affiliate' ? (
                <TwitchGlyph size={16} className="text-[#9146FF]" />
              ) : (
                <Award size={17} strokeWidth={2.25} style={{ color: ICON_COLOR.violet }} />
              )
            }
            label="Account type"
            value={accountType}
          />
        </div>
        )}
      </motion.div>
    </div>
  );

  const lifetimeEl = !lifetimeShown ? null : (
    <div className={`settings-card ${sectionPad}`} style={sectionStyle}>
      <h4 className={`${headMb} flex items-center gap-1.5 text-[12px] font-semibold uppercase tracking-[0.14em] text-textPrimary`}>
        <Tv size={14} className="text-textMuted" /> Lifetime in StreamNook
      </h4>
      <motion.div
        variants={containerV}
        initial="hidden"
        animate="show"
        className={`grid grid-cols-2 ${gridGap} ${compact ? '' : 'sm:grid-cols-3'}`}
      >
        {isOwnProfile && (
          <StatTile
            icon={Coins}
            label="Channel points"
            count={pointsHeld === null && recapPoints === 0 ? undefined : (pointsHeld ?? 0) + recapPoints}
            value={pointsHeld === null && recapPoints === 0 ? 'Unknown' : undefined}
            caption={pointsChannels > 0 ? `across ${pointsChannels} channels` : 'watch a stream to start tracking'}
            color={ICON_COLOR.amber}
          />
        )}
        {!statHidden('messages') && (
          <StatTile
            icon={MessageSquare}
            label="Messages sent"
            count={messages}
            caption="into the void"
            color={ICON_COLOR.green}
          />
        )}
        {!statHidden('streams') && <StatTile icon={Tv} label="Streams watched" count={streams} color={ICON_COLOR.sky} />}
        {subsGifted > 0 && !statHidden('subs_gifted') && (
          <StatTile icon={HandHeart} label="Subs gifted" count={subsGifted} color={ICON_COLOR.violet} />
        )}
        {isOwnProfile && (
          <StatTile
            icon={Gift}
            label="Drops claimed"
            count={dropsClaimed ?? undefined}
            value={dropsClaimed === null ? 'Unknown' : undefined}
            color={ICON_COLOR.violet}
          />
        )}
        {!statHidden('cosmetics') && (
          <StatTile
            icon={Palette}
            label="Cosmetics"
            count={cosmeticsTotal}
            caption={`${seventvPaintCount} ${seventvPaintCount === 1 ? 'paint' : 'paints'}, ${seventvBadgeCount} ${seventvBadgeCount === 1 ? 'badge' : 'badges'}, ${ownedCosmeticsCount} StreamNook`}
            color={ICON_COLOR.rose}
          />
        )}
        {favoriteChannel && !statHidden('favorite_channel') && (
          <StatTile
            icon={Heart}
            label="Favorite channel"
            value={favoriteChannel.channel_name}
            caption={`${Math.round(favoriteChannel.minutes / 60).toLocaleString()}h watched`}
            color={ICON_COLOR.rose}
          />
        )}
        {topPointsChannel && (
          <StatTile
            icon={Coins}
            label="Most points"
            value={topPointsChannel}
            caption="channel points banked"
            color={ICON_COLOR.amber}
          />
        )}
        {streamNookUserNumber !== null && !statHidden('member_rank') && (
          <StatTile
            icon={Sparkles}
            label="Member rank"
            value={`#${streamNookUserNumber.toLocaleString()}`}
            caption={tier?.label || 'StreamNook member'}
            color={ICON_COLOR.slate}
          />
        )}
      </motion.div>
    </div>
  );

  const membershipEl = isOwnProfile ? <MembershipSection userId={userId} /> : null;

  const subsEl = isOwnProfile ? <SubscriptionsSection login={login} /> : null;

  const emotesEl = statHidden('emotes') ? null : <TopEmotesSection userId={userId} />;

  // Accolades - collectible medallions. A big wall, so it anchors the bottom.
  const accoladesEl = statHidden('accolades') || !wall ? null : (
    <div className={`settings-card ${sectionPad}`} style={sectionStyle}>
      <div className={`flex items-center gap-1.5 ${compact ? 'mb-2.5' : 'mb-5'}`}>
        <Trophy size={14} className="text-textMuted" />
        <h4 className="text-[12px] font-semibold uppercase tracking-[0.14em] text-textPrimary">
          Accolades
        </h4>
        <span className="ml-auto text-[11px] tabular-nums text-textMuted">
          {wall.earned} / {wall.total} unlocked
        </span>
      </div>
      <motion.div
        variants={containerV}
        initial="hidden"
        animate="show"
        className={`grid ${compact ? 'grid-cols-6 gap-2.5 sm:grid-cols-8' : 'grid-cols-4 gap-4 sm:grid-cols-6'}`}
      >
        {accoladesShown.map((a) => (
          <AccoladeMedallion key={a.id} a={a} />
        ))}
      </motion.div>
      {compact && lockedCount > 0 && (
        <button
          type="button"
          onClick={() => setShowAllAccolades((v) => !v)}
          className="mt-3 w-full text-[11px] font-medium text-textMuted transition-colors hover:text-textPrimary"
        >
          {showAllAccolades ? 'Show fewer' : `Show all ${wall.total} (${lockedCount} locked)`}
        </button>
      )}
      <div className={`${compact ? 'mt-3' : 'mt-5'} h-1.5 overflow-hidden rounded-full bg-white/[0.06]`}>
        <div
          className="h-full rounded-full bg-accent/70 transition-[width] duration-700 ease-out"
          style={{ width: `${Math.round((wall.earned / Math.max(1, wall.total)) * 100)}%` }}
        />
      </div>
    </div>
  );

  // Compact (public / preview): the hero (hours) leads full width on the solid
  // content surface (identity + tier emblem live in the overlay's hero band
  // above), then the two stat sections side by side (items-stretch keeps them
  // equal height, no L-gap), then the wide grids full width.
  if (compact) {
    return (
      <div className="space-y-3">
        {heroEl}
        {twitchEl && lifetimeEl ? (
          <div className="grid grid-cols-2 items-stretch gap-3">
            {twitchEl}
            {lifetimeEl}
          </div>
        ) : (
          <>
            {twitchEl}
            {lifetimeEl}
          </>
        )}
        {emotesEl}
        {accoladesEl}
      </div>
    );
  }

  // Full (Settings self-view): the original vertical stack.
  return (
    <div className="space-y-6">
      {heroEl}
      {twitchEl}
      {lifetimeEl}
      {membershipEl}
      {subsEl}
      {emotesEl}
      {accoladesEl}
    </div>
  );
};

export default ProfileOverview;
