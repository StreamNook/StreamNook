import { useState, useEffect, useMemo, useRef, useCallback, useSyncExternalStore } from 'react';
import { MessageCircle, UserPlus, UserMinus, Loader2, ChevronDown, ChevronUp, Pencil, X, Gift, Share2, Check, EyeOff, History } from 'lucide-react';
import { isHiddenInScope, withHiddenUser } from '../utils/chatFilters';
import { buildShareUrl } from '../utils/shareLink';
import { providerLabel, type ProviderId } from '../types/providers';
import { motion, AnimatePresence, useScroll, useTransform, useReducedMotion, useMotionValue, animate } from 'framer-motion';
import { setUserNickname, setUserColor } from '../utils/userChatOverrides';
import { Channel, invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useAppStore } from '../stores/AppStore';
import { streamProvider } from '../utils/streamProvider';
import { openBadgesOnStreamNookInMain, openBadgesWithBadgeInMain, openBadgesWithTargetInMain } from '../utils/openBadgesInMain';
import { computePaintStyle, getBadgeImageUrls, getBadgeFallbackUrls, pickPaintLayerImage, queueCosmeticForCaching } from '../services/seventvService';
import { useNameColorAdjust } from '../hooks/useNameColor';
import { FallbackImage } from './FallbackImage';
import { formatIVRDate, formatSubTenure } from '../utils/ivrFormat';
import { Logger } from '../utils/logger';
import {
  getProfileFromMemoryCache,
  getFullProfileWithFallback,
  getCosmeticsWithFallback,
  getCosmeticsFromMemoryCache,
  CachedProfile
} from '../services/cosmeticsCache';
import { Tooltip } from './ui/Tooltip';
import { memberIdFor } from '../utils/memberIdentity';
import { requestMemberAlias } from '../stores/chatUserStore';
import {
  getStreamNookUserNumber,
  subscribeStreamNookRegistryVersion,
  getStreamNookRegistryVersion,
  getOwnedCosmeticSlugs,
  getActiveCosmeticSlug,
  getCosmeticBySlug,
  getCosmeticsVersion,
  subscribeCosmeticsVersion,
} from '../services/supabaseService';
import { resolveCosmeticAsset } from './cosmeticAssets';
import { SLOT_FOR_TYPE, type CosmeticType } from '../services/cosmetics/types';
import { useMemberProfile, type MemberProfilePreview } from './profile/memberProfile';
import { MemberProfileBackdrop } from './profile/MemberProfileBackdrop';
import { MemberProfileHero } from './profile/MemberProfileHero';
import { MemberProfileSections } from './profile/MemberProfileSections';
import { PaintChip, SevenTvProfileButton } from './profile/IdentityChips';
import { StreamNookBadge } from './StreamNookBadge';
import {
  getIdentityWithCache,
  getIdentityFromMemoryCache,
  subscribeIdentityVersion,
  getIdentityVersion,
} from '../services/identityService';
import { EmoteText, buildEmoteNameMap } from '../utils/emoteText';
import { useChannelEmotes } from '../stores/chatConnectionStore';
import { historyKey } from '../utils/chatterIdentity';
import { isChannelScopedTwitchBadge } from '../utils/badgeOrder';
import { usePlatformAccountStore } from '../stores/platformAccountStore';

// messageHistory arrives from two sources:
//   1. The frontend's in-session userMessageHistory Map (full ParsedMessage shape
//      with username/color/badges/tags). Used by the inline-popup fallback path.
//   2. The Rust user_message_history_service via get_user_message_history,
//      which returns a compact UserMessageSummary { id, content, timestamp, color }.
//      Used by the popout window path (the default).
// The reader below prefers top-level `id`/`timestamp` when present and falls back
// to the IRC tags map for the frontend-cache shape.
interface ParsedMessage {
  username?: string;
  content: string;
  color?: string;
  badges?: Array<{ key: string; info: any }>;
  tags?: Map<string, string> | Record<string, string>;
  emotes?: string;
  // Rust UserMessageSummary fields
  id?: string;
  timestamp?: string;
}

/// Mirrors `ModLogsStatus` in src-tauri/src/commands/justlog.rs.
type ModLogsStatus =
  | 'available'
  | 'no_messages'
  | 'no_access'
  | 'auth_required'
  | 'skipped'
  | 'failed';

interface ChatLogsResult {
  messages: { timestamp: string; content: string; id?: string }[];
  mod_logs_status: ModLogsStatus;
}

/** One stat row, rendered only when there is a value. Keeps the platform panels
 *  from growing empty cells for data a platform doesn't publish. */
const ProviderStatRow = ({ label, value }: { label: string; value: string | null }) =>
  value ? (
    <div className="flex items-baseline justify-between gap-3 text-[11px]">
      <span className="text-textSecondary">{label}</span>
      <span className="text-textPrimary font-medium tabular-nums">{value}</span>
    </div>
  ) : null;

interface UserProfileCardProps {
  userId: string;
  username: string;
  displayName: string;
  color: string;
  badges: Array<{ key: string; info: any }>;
  messageHistory: ParsedMessage[];
  onClose: () => void;
  position: { x: number; y: number };
  channelId?: string;
  channelName?: string;
  onStartWhisper?: (user: { id: string; login: string; display_name: string; profile_image_url?: string }) => void;
  isModerator?: boolean;
  /** Whether the VIEWER owns this channel. Broadcasters may act on their own
   *  moderators; moderators may not act on each other. */
  viewerIsBroadcaster?: boolean;
  broadcasterId?: string;
  onPreFillCommand?: (cmd: string) => void;
  /** Which platform this chatter is on. Absent means Twitch, so every existing
   *  caller keeps its exact behaviour. */
  provider?: ProviderId;
  /** Your own card as a live preview while you edit your profile in Settings
   *  (OwnProfilePreview): these edits override what is saved. */
  profilePreview?: MemberProfilePreview | null;
}

/** A Kick chatter's card data, from `kick_user_profile`. */
interface KickProfile {
  user_id?: number | null;
  username: string;
  slug: string;
  avatar_url?: string | null;
  bio?: string | null;
  banner_url?: string | null;
  verified: boolean;
  is_staff: boolean;
  is_channel_owner: boolean;
  is_moderator: boolean;
  is_banned: boolean;
  followers_count?: number | null;
  following_since?: string | null;
  created_at?: string | null;
  subscribed_for: number;
  badges: Array<{ kind: string; text: string; image_url?: string | null; months?: number | null }>;
  socials: Array<[string, string]>;
}

/** A YouTube chatter's channel data, from `youtube_user_profile`. */
interface YouTubeProfile {
  channel_id: string;
  handle?: string | null;
  title?: string | null;
  description?: string | null;
  avatar_url?: string | null;
  banner_url?: string | null;
  subscriber_count?: string | null;
}

/** An early piece of get_user_profile_complete (ProfilePart in user_profile.rs). */
type ProfilePart =
  | { part: 'twitch'; profile: TwitchUserProfile }
  | { part: 'seventv'; cosmetics: SevenTVCosmetics };

interface UserProfileComplete {
  twitch_profile: TwitchUserProfile | null;
  badges: BadgeData;
  seventv_cosmetics: SevenTVCosmetics | null;
  ivr_data: IVRData;
}

interface TwitchUserProfile {
  id: string;
  login: string;
  display_name: string;
  type: string;
  broadcaster_type: string;
  description: string;
  profile_image_url: string;
  offline_image_url: string;
  view_count: number;
  created_at: string;
  /** Channel page header banner. Distinct from offline_image_url (player offline placeholder). */
  banner_image_url: string | null;
}

interface BadgeData {
  display_badges: Badge[];
  earned_badges: Badge[];
  third_party_badges: ThirdPartyBadge[];
}

interface Badge {
  id: string;
  setID: string;
  version: string;
  title: string;
  description: string;
  image1x: string;
  image2x: string;
  image4x: string;
}

interface ThirdPartyBadge {
  id: string;
  provider: string;
  title: string;
  imageUrl: string;
  image1x: string | null;
  image2x: string | null;
  image4x: string | null;
}

interface SevenTVCosmetics {
  paints: SevenTVPaint[];
  badges: SevenTVBadge[];
  /** 7TV animated avatar URL, if the user has set one. 7TV has no banner. */
  avatar_url: string | null;
}

// v4 API Paint structure with layers
interface SevenTVPaint {
  id: string;
  name: string;
  description: string | null;
  selected: boolean;
  data: {
    layers: SevenTVPaintLayer[];
    shadows: SevenTVPaintShadow[];
  };
}

interface SevenTVPaintLayer {
  id: string;
  ty: {
    __typename: string;
    angle?: number;
    repeating?: boolean;
    shape?: string;
    stops?: Array<{ at: number; color: SevenTVColor }>;
    color?: SevenTVColor;
    images?: SevenTVImage[];
  };
  opacity: number;
}

interface SevenTVColor {
  hex: string;
  r: number;
  g: number;
  b: number;
  a: number;
}

interface SevenTVImage {
  url: string;
  mime: string | null;
  size: number | null;
  scale: number | null;
  width: number | null;
  height: number | null;
  frameCount: number | null;
}

interface SevenTVPaintShadow {
  offsetX: number;
  offsetY: number;
  blur: number;
  color: SevenTVColor;
}

interface SevenTVBadge {
  id: string;
  name: string;
  description: string | null;
  selected: boolean;
}

const formatDate = (ds: string) => new Date(ds).toLocaleDateString('en-US', { year: 'numeric', month: 'short', day: 'numeric' });

// Compact relative-time helper. Returns short forms like "3d ago" / "5w ago" /
// "2mo ago" / "1y ago" that fit cleanly in the tight stats panel. Falls back to
// the absolute date if anything fails.
const formatRelative = (ds: string) => {
  try {
    const date = new Date(ds);
    const diffMs = Date.now() - date.getTime();
    const days = Math.floor(diffMs / (1000 * 60 * 60 * 24));
    if (days < 1) return 'today';
    if (days < 7) return `${days}d ago`;
    if (days < 30) return `${Math.floor(days / 7)}w ago`;
    if (days < 365) return `${Math.floor(days / 30)}mo ago`;
    const years = Math.floor(days / 365);
    return years === 1 ? '1y ago' : `${years}y ago`;
  } catch {
    return formatDate(ds);
  }
};

/** Grant or revoke a channel role. The four Tauri commands behind these share a
 *  shape, so one button covers all of them. */
const RoleActionButton = ({
  label,
  tooltip,
  command,
  broadcasterId,
  userId,
  successText,
}: {
  label: string;
  tooltip: string;
  command: string;
  broadcasterId?: string;
  userId: string;
  successText: string;
}) => {
  const [busy, setBusy] = useState(false);
  return (
    <Tooltip content={tooltip} side="top">
      <button
        disabled={busy}
        onClick={async () => {
          if (busy || !broadcasterId) return;
          setBusy(true);
          try {
            const { invoke } = await import('@tauri-apps/api/core');
            await invoke(command, { broadcasterId, userId });
            useAppStore.getState().addToast(successText, 'success');
          } catch (err) {
            Logger.error(`[UserProfileCard] ${command} failed:`, err);
            useAppStore.getState().addToast(typeof err === 'string' ? err : `Could not ${label.toLowerCase()}`, 'error');
          } finally {
            setBusy(false);
          }
        }}
        className="py-1.5 glass-button-secondary text-xs font-semibold text-textSecondary hover:text-textPrimary rounded flex items-center justify-center transition-colors disabled:opacity-50"
      >
        {label}
      </button>
    </Tooltip>
  );
};

/** Muted "(6y ago)" appended to an absolute date, when the user wants both. */
const RelativeSuffix = ({ iso, show }: { iso: string | null | undefined; show: boolean }) => {
  if (!show || !iso) return null;
  return <span className="ml-1 text-textSecondary font-normal">({formatRelative(iso)})</span>;
};

interface IVRData {
  created_at: string | null;
  following_since: string | null;
  status_hidden: boolean;
  is_subscribed: boolean;
  sub_streak: number | null;
  sub_cumulative: number | null;
  is_founder: boolean;
  is_mod: boolean;
  mod_since: string | null;
  is_vip: boolean;
  vip_since: string | null;
  last_broadcast_at: string | null;
  last_broadcast_title: string | null;
  follows_count: number | null;
  banned: boolean;
  chatter_count: number | null;
  sub_tier: string | null;
  sub_type: string | null;
  sub_gifter_login: string | null;
  sub_gifter_display_name: string | null;
  error: string | null;
}

interface NicknameEditorProps {
  userId: string;
  username: string;
  displayName: string;
  // The user's real Twitch color (parsed from the IRC tags). Used as the
  // initial value of the color picker when no override is set, and as the
  // visual baseline after "Reset color" is clicked.
  twitchColor: string;
}

// Normalize whatever shape the IRC color comes in as into a valid #RRGGBB
// hex string that <input type="color"> will accept. Empty / malformed input
// falls back to Twitch's default purple.
// The free, universal StreamNook badge (is_default in the cosmetics catalog).
// StreamNookBadge renders this as its fallback when a member has no cosmetic
// explicitly equipped, so the inactive-thumbnail list must exclude it to avoid
// rendering the same Member badge twice.
const DEFAULT_COSMETIC_SLUG = 'streamnook-default';

/** The in-app card's width, and a StreamNook member's wider card. Must match
 *  the `w-[440px]` / `w-[760px]` on the card root and the popout window sizes
 *  in utils/openProfilePopup.ts. */
const PROFILE_CARD_WIDTH = 440;
const MEMBER_CARD_WIDTH = 760;

const TWITCH_DEFAULT_COLOR = '#9147FF';
function normalizeHex(input: string | null | undefined): string {
  if (!input) return TWITCH_DEFAULT_COLOR;
  const v = input.trim();
  if (/^#[0-9a-fA-F]{6}$/.test(v)) return v.toLowerCase();
  if (/^[0-9a-fA-F]{6}$/.test(v)) return `#${v.toLowerCase()}`;
  return TWITCH_DEFAULT_COLOR;
}

// Friendly labels + display order for third-party chat-client badge providers.
// Keyed by the CANONICAL lowercase provider id (BadgeProvider::as_key in Rust)
// and matched case-insensitively, because one provider string here legitimately
// isn't lowercase: the BTTV Pro badge is tagged 'BTTV' on the client so that
// `${provider}:${id}` reproduces BTTV_PRO_LOADOUT_KEY, which is already persisted
// in members' loadouts as "BTTV:bttv-pro" and can't be renamed without orphaning
// it. We group the badges under these per-provider headers instead of one flat
// "Other" pile so each badge keeps its provenance, mirroring the Attainables
// "Chat Clients" gallery. Any provider not listed here falls through to a
// catch-all "Other" group, so a new Rust-side provider can't silently vanish.
const THIRD_PARTY_PROVIDER_GROUPS: { key: string; label: string }[] = [
  { key: 'ffz', label: 'FrankerFaceZ' },
  { key: 'bttv', label: 'BetterTTV' },
  { key: 'chatterino', label: 'Chatterino' },
  { key: 'chatsen', label: 'Chatsen' },
  { key: 'chatty', label: 'Chatty' },
  { key: 'dankchat', label: 'DankChat' },
  { key: 'homies', label: 'Homies' },
  { key: 'moltorino', label: 'Moltorino' },
];
const providerGroupKey = (provider: unknown): string =>
  typeof provider === 'string' ? provider.toLowerCase() : '';

// Inline editor for the nickname + color overrides we expose on each user.
// Reads the current override from the settings store on mount (lazy init)
// and on userId change. Nickname saves on blur or Enter; color saves on the
// native color picker's change event.
const NicknameEditor: React.FC<NicknameEditorProps> = ({ userId, username, displayName, twitchColor }) => {
  const readCurrentNickname = () =>
    useAppStore.getState().settings.chat_customization?.user_overrides?.[userId]?.nickname ?? '';
  const readCurrentColor = () =>
    useAppStore.getState().settings.chat_customization?.user_overrides?.[userId]?.color ?? '';

  const [draft, setDraft] = useState<string>(readCurrentNickname);
  const [savedNickname, setSavedNickname] = useState<string>(readCurrentNickname);
  const [isEditing, setIsEditing] = useState<boolean>(false);
  const [savedColor, setSavedColor] = useState<string>(readCurrentColor);
  const inputRef = useRef<HTMLInputElement | null>(null);

  const fallbackColor = normalizeHex(twitchColor);
  const pickerValue = normalizeHex(savedColor || twitchColor);
  const hasColorOverride = savedColor.trim().length > 0;

  useEffect(() => {
    const nextNickname = readCurrentNickname();
    const nextColor = readCurrentColor();
    setDraft(nextNickname);
    setSavedNickname(nextNickname);
    setSavedColor(nextColor);
    setIsEditing(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [userId]);

  useEffect(() => {
    if (isEditing) {
      inputRef.current?.focus();
      inputRef.current?.select();
    }
  }, [isEditing]);

  const commitNickname = () => {
    const next = draft.trim();
    if (next === savedNickname) {
      setIsEditing(false);
      return;
    }
    setUserNickname(userId, username, next.length > 0 ? next : null);
    setSavedNickname(next);
    setIsEditing(false);
  };

  const cancelNickname = () => {
    setDraft(savedNickname);
    setIsEditing(false);
  };

  const clearNickname = () => {
    setUserNickname(userId, username, null);
    setSavedNickname('');
    setDraft('');
    setIsEditing(false);
  };

  const commitColor = (next: string) => {
    const normalized = normalizeHex(next);
    if (normalized === savedColor) return;
    setUserColor(userId, username, normalized);
    setSavedColor(normalized);
  };

  const resetColor = () => {
    setUserColor(userId, username, null);
    setSavedColor('');
  };

  return (
    <div className="space-y-1">
      {/* Nickname row */}
      {!isEditing ? (
        <div className="flex items-center gap-2 px-1 py-1 text-xs text-textSecondary">
          <span className="text-textMuted">Nickname:</span>
          {savedNickname ? (
            <>
              <span className="text-textPrimary font-medium">{savedNickname}</span>
              <button
                onClick={() => setIsEditing(true)}
                className="ml-auto p-1 hover:text-textPrimary transition-colors"
                aria-label="Edit nickname"
              >
                <Pencil size={12} />
              </button>
              <button
                onClick={clearNickname}
                className="p-1 hover:text-error transition-colors"
                aria-label="Clear nickname"
              >
                <X size={12} />
              </button>
            </>
          ) : (
            <button
              onClick={() => setIsEditing(true)}
              className="ml-auto inline-flex items-center gap-1 text-textSecondary hover:text-textPrimary transition-colors"
            >
              <Pencil size={12} />
              <span>Set nickname</span>
            </button>
          )}
        </div>
      ) : (
        <div className="flex items-center gap-2 px-1 py-1">
          <span className="text-xs text-textMuted">Nickname:</span>
          <input
            ref={inputRef}
            type="text"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onBlur={commitNickname}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault();
                commitNickname();
              } else if (e.key === 'Escape') {
                e.preventDefault();
                cancelNickname();
              }
            }}
            placeholder={displayName}
            maxLength={32}
            className="flex-1 glass-input text-textPrimary text-xs px-2 py-1"
            spellCheck={false}
          />
        </div>
      )}

      {/* Color row */}
      <div className="flex items-center gap-2 px-1 py-1 text-xs text-textSecondary">
        <span className="text-textMuted">Color:</span>
        <input
          type="color"
          value={pickerValue}
          onChange={(e) => commitColor(e.target.value)}
          className="w-7 h-7 rounded cursor-pointer bg-transparent border border-borderSubtle"
          aria-label="Override chat color"
        />
        {hasColorOverride ? (
          <>
            <span className="font-mono text-textPrimary">{savedColor}</span>
            <button
              onClick={resetColor}
              className="ml-auto p-1 hover:text-error transition-colors"
              aria-label="Reset color"
            >
              <X size={12} />
            </button>
          </>
        ) : (
          <span className="font-mono">{fallbackColor}<span className="text-textMuted/70 ml-1">(Twitch)</span></span>
        )}
      </div>
    </div>
  );
};

// Local chat filter controls: hide this user's messages in this channel or in
// every channel. Purely local (nothing is sent to any platform); the gate runs
// at chat-store ingest, so it applies to the widget, MultiChat panes and the
// overlay feed alike. State reads live from settings so the buttons flip to
// "Unhide" without reopening the card.
const ChatFilterActions = ({
  username,
  displayName,
  provider,
  channel,
}: {
  username: string;
  displayName: string;
  provider: ProviderId;
  channel: string;
}) => {
  const cf = useAppStore((s) => s.settings.chat_filters);
  const scope = channel ? ({ provider, channel } as const) : null;
  const hiddenHere = scope ? isHiddenInScope(cf, username, scope) : false;
  const hiddenGlobal = isHiddenInScope(cf, username, 'global');
  const toggle = (target: 'channel' | 'global') => {
    const st = useAppStore.getState();
    const next = withHiddenUser(
      st.settings.chat_filters,
      username,
      target === 'global' ? 'global' : scope!,
      target === 'global' ? !hiddenGlobal : !hiddenHere,
    );
    st.updateSettings({ ...st.settings, chat_filters: next });
  };
  return (
    <div className="pt-3 border-t border-borderSubtle">
      <div className="flex items-center gap-1.5 mb-2.5">
        <EyeOff size={13} className="text-textSecondary" />
        <span className="text-[10px] uppercase font-bold text-textSecondary tracking-wider">Chat Filters</span>
      </div>
      <div className="grid grid-cols-2 gap-2">
        <Tooltip content={`Only affects your own chat, only in this channel`} side="top">
          <button
            onClick={() => scope && toggle('channel')}
            disabled={!scope}
            className={`w-full py-1.5 glass-button text-xs font-semibold rounded flex items-center justify-center transition-colors border disabled:opacity-40 ${
              hiddenHere ? 'text-white bg-accent/20 border-accent/30' : 'text-white/70 hover:text-white'
            }`}
          >
            {hiddenHere ? 'Unhide here' : 'Hide in this channel'}
          </button>
        </Tooltip>
        <Tooltip content={`Only affects your own chat, in every channel`} side="top">
          <button
            onClick={() => toggle('global')}
            className={`w-full py-1.5 glass-button text-xs font-semibold rounded flex items-center justify-center transition-colors border ${
              hiddenGlobal ? 'text-white bg-accent/20 border-accent/30' : 'text-white/70 hover:text-white'
            }`}
          >
            {hiddenGlobal ? 'Unhide everywhere' : 'Hide everywhere'}
          </button>
        </Tooltip>
      </div>
      {(hiddenHere || hiddenGlobal) && (
        <p className="mt-2 text-[11px] text-textSecondary">
          New messages from {displayName || username} won't appear in your chat.
        </p>
      )}
    </div>
  );
};

const UserProfileCard = ({
  userId,
  username,
  displayName,
  color,
  // Badges from the chat message this card was opened from. Synchronous, so it
  // can gate moderation UI without a flash; see targetIsModerator below.
  badges: messageBadges,
  messageHistory,
  onClose,
  position,
  channelId: propChannelId,
  channelName: propChannelName,
  onStartWhisper,
  isModerator = false,
  viewerIsBroadcaster = false,
  broadcasterId,
  onPreFillCommand,
  provider = 'twitch',
  profilePreview = null,
}: UserProfileCardProps) => {
  const [profileData, setProfileData] = useState<UserProfileComplete | null>(null);
  // Per-platform card data. Only one is ever populated; the shell is shared.
  const [kickProfile, setKickProfile] = useState<KickProfile | null>(null);
  const [youtubeProfile, setYoutubeProfile] = useState<YouTubeProfile | null>(null);
  const [cachedProfile, setCachedProfile] = useState<CachedProfile | null>(null);
  // Pieces Rust sends ahead of the whole profile (see ProfilePart in
  // user_profile.rs): Helix answers in ~200 ms, well before IVR and the badge
  // lookups, so the avatar and bio paint first instead of a blank card.
  // Keyed by the chatter they describe, so switching chatters needs no reset:
  // a stale entry simply stops matching.
  const [early, setEarly] = useState<{ userId: string; twitch?: TwitchUserProfile; seventv?: SevenTVCosmetics }>({ userId: '' });
  const earlyTwitch = early.userId === userId ? early.twitch ?? null : null;
  const earlySeventv = early.userId === userId ? early.seventv ?? null : null;
  // Which chatter's Twitch profile has fully settled (both halves answered).
  const [settledFor, setSettledFor] = useState<string | null>(null);
  // What chat already resolved for this chatter (the worn paint and 7TV
  // badge), so the name is painted on the first frame.
  const wornCosmetics = useMemo(
    () => (provider === 'twitch' ? getCosmeticsFromMemoryCache(userId) : null),
    [userId, provider],
  );
  const [isLoadingProfile, setIsLoadingProfile] = useState(true);
  // Twitch chatters load through the two-half fetch below, settled per chatter;
  // the other platforms keep their own flag.
  const profileLoading = provider === 'twitch' ? settledFor !== userId : isLoadingProfile;
  // BetterTTV Pro loyalty badge. Resolved separately from the main profile
  // fetch because BTTV only serves Pro badges over a WebSocket and non-Pro
  // users never reply (so the lookup can take up to its timeout). Keeping it
  // off the critical path lets the card render immediately and the badge pop in
  // when/if it resolves. See bttv_pro_service.rs.
  const [bttvProBadge, setBttvProBadge] = useState<{ url: string; started_at: string | null; glow: boolean } | null>(null);
  // Which half opens first, per the Chat setting. Off (the default) opens the
  // profile body, with badges and stats; on lands on the chat history in one
  // click. Either way the header switches between the two. The history fetch is
  // keyed on this, so opening on messages also starts fetching on mount instead
  // of waiting for a second click.
  const [showMessages, setShowMessages] = useState(
    () => useAppStore.getState().settings.chat_design?.user_card_opens_messages === true,
  );
  // Which rows this card shows. Subscribed (not a one-shot read) so toggling a
  // row in Settings updates an open card.
  const cardPrefs = useAppStore((s) => s.settings.user_card);
  // Pronouns (opt-in) and the private note, both served by Rust.
  const [pronouns, setPronouns] = useState<string | null>(null);
  useEffect(() => {
    if (!cardPrefs?.show_pronouns || !username) return;
    let cancelled = false;
    void invoke<string | null>('get_user_pronouns', { login: username })
      .then((p) => {
        if (!cancelled) setPronouns(p);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [cardPrefs?.show_pronouns, username]);
  const [note, setNote] = useState('');
  const [noteSaved, setNoteSaved] = useState<'idle' | 'saving' | 'saved'>('idle');
  const noteLoadedRef = useRef<string>('');
  useEffect(() => {
    if (cardPrefs?.show_notes === false || !userId) return;
    let cancelled = false;
    void invoke<{ note: string } | null>('get_user_note', { userId })
      .then((n) => {
        if (cancelled) return;
        const text = n?.note ?? '';
        noteLoadedRef.current = text;
        setNote(text);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [cardPrefs?.show_notes, userId]);
  const saveNote = async () => {
    if (note === noteLoadedRef.current) return;
    setNoteSaved('saving');
    try {
      await invoke('set_user_note', { userId, note });
      noteLoadedRef.current = note;
      setNoteSaved('saved');
      window.setTimeout(() => setNoteSaved('idle'), 1500);
    } catch {
      setNoteSaved('idle');
    }
  };
  const showRelative = cardPrefs?.show_relative_time !== false;
  // Twitch-suspended account. Worth marking because their messages stay in
  // chat history long after the account is gone.
  const isBanned = !!profileData?.ivr_data?.banned;
  // Resolved by the cosmetics cache alongside paints/badges. The Rust profile
  // path asks 7TV for the id but drops it, so read it from the cache.
  const seventvUserId = cachedProfile?.seventvCosmetics?.seventvUserId;

  // Whether the person this card is about is themselves privileged here. Read
  // from the badges on the message we were opened from: synchronous and
  // first-party, unlike ivrData, which arrives later (so the buttons would
  // flash) and comes from a third-party service (so moderation UI would depend
  // on its uptime).
  const targetIsBroadcaster =
    (!!broadcasterId && userId === broadcasterId) ||
    (messageBadges ?? []).some((b) => String(b.key ?? '').startsWith('broadcaster/'));
  const targetIsModerator = (messageBadges ?? []).some((b) =>
    String(b.key ?? '').startsWith('moderator/'),
  );
  // Twitch rejects a moderator acting on another moderator or on the channel
  // owner. The broadcaster keeps full reach over their own channel, so only the
  // moderator case is blocked here; nothing the broadcaster could legitimately
  // do is taken away.
  const blockedTarget = !viewerIsBroadcaster && (targetIsModerator || targetIsBroadcaster);

  // The provider zones need the same protection. Without it, a moderator's own
  // card offers "Ban" pointed at themselves — one click from being banned from a
  // channel they moderate. Kick reports the card's subject in the same payload
  // that told us we can moderate, so compare against the connected account.
  const connectedKickName = usePlatformAccountStore((s) =>
    s.kick.connected ? (s.kick.name?.toLowerCase() ?? null) : null,
  );
  const connectedYouTubeName = usePlatformAccountStore((s) =>
    s.youtube.connected ? (s.youtube.name?.toLowerCase() ?? null) : null,
  );
  const targetIsSelf =
    provider === 'kick'
      ? !!connectedKickName &&
        [username, kickProfile?.slug, kickProfile?.username].some(
          (n) => !!n && String(n).toLowerCase() === connectedKickName,
        )
      : provider === 'youtube'
        ? !!connectedYouTubeName &&
          [displayName, username].some(
            (n) => !!n && String(n).toLowerCase() === connectedYouTubeName,
          )
        : false;

  // First saved reason is the one we prefill; the rest are just there to copy.
  const savedBanReasons = useAppStore((s) => s.settings.moderation?.saved_ban_reasons);
  const defaultBanReason = savedBanReasons?.[0]?.trim() || '';
  // In-session messages that arrive WHILE the card is open, polled from the Rust
  // history service so the timeline stays live (works across the popout-window
  // boundary — the service is a shared backend singleton). Same shape as the
  // messageHistory prop; deduped against it by id in the render.
  const [liveMessages, setLiveMessages] = useState<ParsedMessage[]>([]);
  // Historical chat logs pulled from the merged Twitch GQL + Justlog + Robotty
  // pipeline in Rust. Each entry carries the Twitch IRC message UUID in `id`
  // (when available) which the frontend uses to dedupe against in-session
  // messages — without that cross-source dedupe, the same message can appear
  // both as in-session (we saw it land in chat) and as historical (Robotty or
  // Twitch's MessageBufferChatHistory included it), showing as a duplicate.
  const [historicalMessages, setHistoricalMessages] = useState<{ timestamp: string; content: string; id?: string }[]>([]);
  const [historicalLoading, setHistoricalLoading] = useState(false);
  const [deepLoading, setDeepLoading] = useState(false);
  // Fetch guard as a REF, not state: if it were state and in the effect's deps,
  // setting it would re-run the effect, whose cleanup sets `cancelled = true`,
  // which then discards the in-flight fetch result. (That was the bug where deep
  // history loaded but never rendered.) A ref set doesn't trigger a re-render.
  const historicalAttemptedRef = useRef(false);
  const [historicalChannelLogged, setHistoricalChannelLogged] = useState(true);
  // Why Twitch's own deep archive contributed what it did. Twitch only serves it
  // to moderators of the channel (anyone may always read their OWN history), so
  // 'no_access' is the ordinary outcome in most channels and needs saying out
  // loud — otherwise a bare panel reads as StreamNook being broken.
  const [modLogsStatus, setModLogsStatus] = useState<ModLogsStatus | null>(null);

  // Re-render when the StreamNook registry updates so the #N chip appears as soon as it loads
  useSyncExternalStore(subscribeStreamNookRegistryVersion, getStreamNookRegistryVersion, getStreamNookRegistryVersion);
  // Re-render when the cosmetics registry updates so this user's owned-but-not-active
  // StreamNook badges appear / refresh as ownership changes (e.g. a grant from a
  // streamnook.app purchase lands while the popup is open).
  useSyncExternalStore(subscribeCosmeticsVersion, getCosmeticsVersion, getCosmeticsVersion);
  // `userId` is the id ON THE CHATTER'S PLATFORM, and `provider` says which. For
  // a Kick chatter that is a bare Kick number, from the same range Twitch ids
  // come from, so passing it straight to a membership lookup shows an unrelated
  // Twitch member's number and badges on a stranger's card. Every StreamNook
  // lookup on this card goes through `cardMemberId` instead: the member's Twitch
  // id when this chat identity has been claimed, null when it has not.
  const cardChatKey = provider === 'twitch' ? userId : `${provider}:${userId}`;
  const cardMemberId = memberIdFor(cardChatKey);
  // A card can be the first place this chatter's claim is wanted. Asking is
  // idempotent and batched, and the answer repaints the card when it lands.
  useEffect(() => {
    requestMemberAlias(cardChatKey);
  }, [cardChatKey]);
  const streamNookUserNumber = getStreamNookUserNumber(cardMemberId);
  // Only enumerate owned StreamNook cosmetics for users who are actually in the
  // registry. `getOwnedCosmeticSlugs` includes every `is_default` cosmetic for
  // ANY userId by design (so the picker can preview defaults), which means
  // calling it for a non-StreamNook user returns the defaults set — and any
  // surface that renders those would falsely show the default badge to people
  // who aren't members. Gate at the source: empty list when not registered.
  const isStreamNookMember = streamNookUserNumber !== null;
  const ownedStreamNookSlugs = useMemo(
    () => (isStreamNookMember && cardMemberId ? Array.from(getOwnedCosmeticSlugs(cardMemberId)) : []),
    [cardMemberId, isStreamNookMember, getCosmeticsVersion()],
  );
  const activeStreamNookSlug = isStreamNookMember && cardMemberId ? getActiveCosmeticSlug(cardMemberId) : null;
  // A StreamNook member's card IS their StreamNook profile, the only one: their
  // hero, atmosphere, relics and overview, with this card's Twitch sections,
  // actions and mod tools added. Hovering their name in chat decodes their
  // number (MemberReveal); clicking it opens this.
  const isMemberCard = isStreamNookMember && !!cardMemberId;
  const memberView = useMemberProfile(isMemberCard ? cardMemberId : null, profilePreview, !!profilePreview);
  // The <StreamNookBadge> slot below always renders the member's PRIMARY mark:
  // their active cosmetic when it has art (bundled or cloud-served, the same
  // resolution StreamNookBadge uses), otherwise the default "StreamNook Member"
  // badge. Whatever that slot shows must be excluded from the other thumbnails,
  // or it renders twice (the double-Member-badge bug for members who never
  // equipped one).
  const shownPrimarySlug =
    activeStreamNookSlug && resolveCosmeticAsset(getCosmeticBySlug(activeStreamNookSlug), { chatSize: true })
      ? activeStreamNookSlug
      : DEFAULT_COSMETIC_SLUG;
  // Every other StreamNook BADGE the member owns (bought, earned from events and
  // milestones, or granted), cloud-served ones included, in catalog order. The
  // other kinds show in their own places: the frame on the banner, the
  // atmosphere as the backdrop, relics in their strip.
  const inactiveOwnedSlugs = useMemo(() => {
    return ownedStreamNookSlugs
      .filter((s) => s !== shownPrimarySlug)
      .map((s) => ({ slug: s, cosmetic: getCosmeticBySlug(s) }))
      .filter((entry): entry is { slug: string; cosmetic: NonNullable<ReturnType<typeof getCosmeticBySlug>> } => {
        if (!entry.cosmetic) return false;
        const slot = SLOT_FOR_TYPE[entry.cosmetic.kind as CosmeticType];
        return slot === undefined || slot === 'badge';
      })
      .map((entry) => ({ ...entry, asset: resolveCosmeticAsset(entry.cosmetic, { chatSize: true }) }))
      .filter((entry): entry is typeof entry & { asset: string } => !!entry.asset)
      .sort((a, b) => a.cosmetic.sort_order - b.cosmetic.sort_order);
  }, [ownedStreamNookSlugs, shownPrimarySlug]);
  const streamNookBadgeCount = isStreamNookMember ? 1 + inactiveOwnedSlugs.length : 0;
  if (import.meta.env.DEV && typeof window !== 'undefined') {
    (window as any).__snProfileDebug = { userId, username, displayName, streamNookUserNumber };
  }

  const [isDragging, setIsDragging] = useState(false);
  const [dragOffset, setDragOffset] = useState({ x: 0, y: 0 });
  const [cardPosition, setCardPosition] = useState({ x: 0, y: 0 });
  const cardRef = useRef<HTMLDivElement>(null);

  // Scroll-driven collapsing header. As the body scrolls, the banner shrinks +
  // darkens and the avatar scales down and tucks up toward the now-compact
  // header, so it gives way to content instead of staying fixed. Pure
  // framer-motion useScroll -> useTransform (GPU transforms + one animated
  // height, no scroll listeners). The expanded (scrollY 0) values are pinned to
  // the original layout so nothing changes until you actually scroll, and
  // prefers-reduced-motion keeps everything in the expanded state.
  const scrollBodyRef = useRef<HTMLDivElement>(null);
  // Whether the message timeline is pinned to the newest entry. True while the
  // user is at/near the bottom so live messages keep it scrolled to the latest;
  // set false once they scroll up to read older history, so we don't yank them.
  const stickBottomRef = useRef(true);
  const prefersReducedMotion = useReducedMotion();
  const { scrollY } = useScroll({ container: scrollBodyRef });
  const HEADER_COLLAPSE_PX = 90; // body scroll distance to fully collapse
  // Collapse runs off a single 0->1 progress so two triggers can share it: the
  // body scrolling, and the recent-messages view opening (which forces a full
  // collapse so the message list gets the room — the avatar no longer overhangs
  // there). messagesForce eases between 0 and 1 on that toggle; collapseProgress
  // is whichever trigger is further along.
  const scrollProgress = useTransform(scrollY, [0, HEADER_COLLAPSE_PX], [0, 1], { clamp: true });
  const messagesForce = useMotionValue(0);
  const collapseProgress = useTransform(() => Math.max(scrollProgress.get(), messagesForce.get()));
  useEffect(() => {
    const controls = animate(messagesForce, showMessages ? 1 : 0, { duration: 0.3, ease: [0.22, 1, 0.36, 1] });
    return () => controls.stop();
  }, [showMessages, messagesForce]);
  const bannerHeight = useTransform(collapseProgress, [0, 1], [96, 56]);
  const bannerZoom = useTransform(collapseProgress, [0, 1], [1, 1.1]);
  const bannerScrim = useTransform(collapseProgress, [0, 1], [0, 0.45]);
  // Banner image blurs as it collapses so the compact name row reads on top of it.
  const bannerBlur = useTransform(collapseProgress, [0, 1], ['blur(0px)', 'blur(5px)']);
  // 0.6 shrink + a -44px lift lands the 80px avatar centered vertically in the
  // 56px collapsed banner (transform-origin keeps it pinned to the left edge).
  const avatarScale = useTransform(collapseProgress, [0, 1], [1, 0.6]);
  const avatarLift = useTransform(collapseProgress, [0, 1], [0, -44]);
  // Compact name+chips row: fades in over the back half of the collapse (once the
  // body's full identity block has scrolled up under the header) and only takes
  // pointer events once it's actually visible, so its chips aren't click targets
  // while invisible in the expanded state.
  const collapsedNameOpacity = useTransform(collapseProgress, [0.6, 1], [0, 1]);
  const collapsedNamePointer = useTransform(collapseProgress, (v) => (v > 0.78 ? 'auto' : 'none'));
  // Body top padding tightens in the messages view since the avatar isn't
  // overhanging there; tied to messagesForce only so plain scrolling doesn't
  // pull content up while the overhang still needs that space.
  const bodyPadTop = useTransform(messagesForce, [0, 1], [48, 12]);

  // Follow state
  const [isFollowing, setIsFollowing] = useState<boolean | null>(null);
  const [followLoading, setFollowLoading] = useState(false);
  // Share confirmation (copies streamnook.app/w/<login>).
  const [shareCopied, setShareCopied] = useState(false);

  // Get channel context
  const getChannelContext = useCallback(() => {
    if (propChannelId && propChannelName) {
      return { channelId: propChannelId, channelName: propChannelName };
    }
    const currentStream = useAppStore.getState().currentStream;
    // Deliberately NO fallback to the viewed user's own id/login. Substituting
    // them silently asks "what did this user say in their OWN channel", which
    // returns nothing for almost everyone, resolves badges in the wrong channel
    // (making every viewer read as the broadcaster), and is indistinguishable
    // from a genuine empty result. An unknown channel is better reported as
    // unknown: the emote hook no-ops on an empty channel, and the history
    // command reports `skipped` so the card can say so.
    return {
      channelId: currentStream?.user_id ?? '',
      channelName: currentStream?.user_login ?? '',
    };
  }, [propChannelId, propChannelName]);

  // Channel emote set (Twitch native + FFZ/BTTV/7TV), reused from the shared
  // cache the chat renderer fills. In the popout window that cache starts empty,
  // so the hook lazily fetches and re-renders when it lands — emotes pop in.
  // buildEmoteNameMap gives the name -> emote lookup EmoteText needs to swap
  // emote words for images in the message timeline.
  const { channelId: emoteChannelId, channelName: emoteChannelName } = getChannelContext();
  // The provider travels with the channel: `getChannelContext` falls back to the
  // current stream, which on YouTube/Kick yields that platform's id, and the hook
  // would otherwise default to Twitch and look it up on Helix.
  const emoteProvider = streamProvider(useAppStore.getState().currentStream);
  const channelEmotes = useChannelEmotes(emoteChannelName, emoteChannelId, emoteProvider);
  const emoteNameMap = useMemo(() => buildEmoteNameMap(channelEmotes), [channelEmotes]);

  // Load cached cosmetics INSTANTLY (synchronous), then fetch fresh data
  useEffect(() => {
    const { channelId, channelName } = getChannelContext();

    // 1. Try to load from memory cache IMMEDIATELY (no await, synchronous)
    const cached = getProfileFromMemoryCache(userId);
    if (cached) {
      Logger.debug('[UserProfileCard] Instant load from cache:', cached);
      setCachedProfile(cached);
    }

    // Non-Twitch chatters take a different path entirely. `get_user_profile_complete`
    // is Helix + IVR + 7TV — every one of them answers about TWITCH accounts, so
    // handing it a Kick numeric id or a YouTube UC id returns a stranger's profile
    // or nothing. Each platform has its own command with its own shape.
    if (provider !== 'twitch') {
      const controller = new AbortController();
      const fetchProviderProfile = async () => {
        setIsLoadingProfile(true);
        // 7TV cosmetics are a 7TV-ACCOUNT property, not a Twitch one: a linked Kick
        // connection resolves the same paint and badges the person wears anywhere
        // else. This branch fetched only the platform profile, so a Kick chatter
        // rendered painted in chat and flat on their own card. Started HERE rather
        // than awaited after the profile so the card does not get slower.
        //
        // Kick and YouTube, matching the chat row: 7TV knows YouTube as GOOGLE and
        // resolves a raw channel id, so `chatUserStore.addUser` resolves both, and
        // the card has to as well or a painted name in chat opens a flat card.
        // TikTok stays out for the same reason chat leaves it out: 7TV has no
        // TikTok platform at all.
        const cosmeticsId = `${provider}:${userId}`;
        const cosmeticsPromise =
          provider === 'kick' || provider === 'youtube'
            ? getCosmeticsWithFallback(cosmeticsId).catch(() => null)
            : null;
        try {
          if (provider === 'kick') {
            const p = await invoke<KickProfile>('kick_user_profile', {
              room: channelName || '',
              username,
            });
            if (!controller.signal.aborted) setKickProfile(p);
          } else if (provider === 'youtube') {
            const p = await invoke<YouTubeProfile>('youtube_user_profile', {
              channelId: userId,
            });
            if (!controller.signal.aborted) setYoutubeProfile(p);
          }
        } catch (error) {
          Logger.warn(`[UserProfileCard] ${provider} profile failed:`, error);
        } finally {
          if (!controller.signal.aborted) setIsLoadingProfile(false);
        }
        // Feed the cosmetics into the SAME `cachedProfile.seventvCosmetics` slot the
        // Twitch card already renders from, so the paint and 7TV badge rows need no
        // provider branch of their own.
        const cosmetics = await cosmeticsPromise;
        if (cosmetics && !controller.signal.aborted) {
          setCachedProfile((prev) => ({
            userId: cosmeticsId,
            username,
            twitchBadges: [],
            thirdPartyBadges: [],
            ...(prev ?? {}),
            seventvCosmetics: cosmetics,
            lastUpdated: Date.now(),
          }));
        }
      };
      void fetchProviderProfile();
      // Closing the card, or clicking a different chatter, must not leave a
      // response in flight that writes into an unmounted card.
      return () => controller.abort();
    }

    // 2. The Rust profile (Helix, IVR, badges, 7TV) and the cosmetics profile,
    // each rendered the moment it lands rather than both waiting on the slower.
    // Rust also streams its Helix and 7TV answers ahead of the rest.
    let alive = true;
    const onPart = new Channel<ProfilePart>();
    onPart.onmessage = (part) => {
      if (!alive) return;
      setEarly((prev) => {
        const base = prev.userId === userId ? prev : { userId };
        return part.part === 'twitch' ? { ...base, twitch: part.profile } : { ...base, seventv: part.cosmetics };
      });
    };
    const rustHalf = invoke<UserProfileComplete>('get_user_profile_complete', {
      userId,
      username,
      channelId,
      channelName,
      onPart,
    })
      .then((profile) => {
        if (alive) setProfileData(profile);
      })
      .catch((error) => Logger.error('Failed to fetch user profile:', error));
    const cosmeticsHalf = getFullProfileWithFallback(userId, username, channelId, channelName)
      .then((profile) => {
        if (alive) setCachedProfile(profile);
      })
      .catch((error) => Logger.warn('[UserProfileCard] cosmetics profile failed:', error));
    void Promise.allSettled([rustHalf, cosmeticsHalf]).then(() => {
      if (alive) setSettledFor(userId);
    });
    return () => {
      alive = false;
    };
  }, [userId, username, getChannelContext]);

  // Resolve the BetterTTV Pro loyalty badge in parallel (non-blocking). Pops
  // into the BetterTTV badge group when/if it resolves; silent for non-Pro users.
  useEffect(() => {
    let cancelled = false;
    setBttvProBadge(null);
    if (!userId) return;
    invoke<{ url: string; started_at: string | null; glow: boolean } | null>('get_bttv_pro_badge', { userId })
      .then((badge) => { if (!cancelled) setBttvProBadge(badge); })
      .catch((err) => Logger.debug('[UserProfileCard] BTTV Pro lookup failed:', err));
    return () => { cancelled = true; };
  }, [userId]);

  // Lazily fetch the user's historical chat logs from Justlog the first time
  // the messages view is opened. We don't pre-fetch because most popup opens
  // never expand messages — paying the request cost upfront would be wasteful.
  useEffect(() => {
    if (!showMessages || historicalAttemptedRef.current) return;
    const { channelName, channelId } = getChannelContext();
    if (!channelName || !username) return;

    let cancelled = false;
    historicalAttemptedRef.current = true;
    setHistoricalLoading(true);
    setDeepLoading(true);
    // Failsafe so neither phase can leave a spinner stuck if IPC wedges. Whatever
    // is already shown stays; we just stop implying more is still coming.
    const failsafe = setTimeout(() => {
      if (!cancelled) {
        setHistoricalLoading(false);
        setDeepLoading(false);
      }
    }, 20000);

    // Merge new messages into the historical set, de-duping by Twitch message id
    // so the fast and deep phases (which overlap on recent messages) don't stack
    // duplicates. Messages without an id fall through to the render's own dedupe.
    const mergeHistorical = (
      prev: { timestamp: string; content: string; id?: string }[],
      incoming: { timestamp: string; content: string; id?: string }[],
    ) => {
      if (!incoming || incoming.length === 0) return prev;
      const seen = new Set(prev.map((m) => m.id).filter(Boolean));
      const add = incoming.filter((m) => !m.id || !seen.has(m.id));
      return add.length ? prev.concat(add) : prev;
    };

    (async () => {
      const { invoke } = await import('@tauri-apps/api/core');

      // Fast phase: Twitch buffer + robotty + mod logs. Returns in ~1s, so the
      // card fills right away. channelId + userId enable the Twitch GQL sources.
      const fastP = invoke<ChatLogsResult>(
        'fetch_user_chat_logs',
        { channel: channelName, username, channelId, userId },
      )
        .then((result) => {
          const messages = result?.messages ?? [];
          Logger.info(
            `[UserProfileCard] fast logs: channel=${channelName} user=${username} -> ${messages.length} msgs (modLogs: ${result?.mod_logs_status})`,
          );
          if (!cancelled) {
            setModLogsStatus(result?.mod_logs_status ?? null);
            setHistoricalMessages((prev) => mergeHistorical(prev, messages));
            if (messages.length > 0) setHistoricalChannelLogged(true);
          }
        })
        .catch((e) => Logger.warn('[UserProfileCard] fast chat fetch failed:', e))
        .finally(() => {
          if (!cancelled) setHistoricalLoading(false);
        });

      // Deep phase: Justlog (via best-logs, with proxy fallback). Can take several
      // seconds; it merges in when it lands without holding up the card.
      const deepP = invoke<{ timestamp: string; content: string; id?: string }[]>(
        'fetch_user_deep_logs',
        { channel: channelName, username },
      )
        .then((messages) => {
          Logger.info(
            `[UserProfileCard] deep logs: channel=${channelName} user=${username} -> ${messages.length} msgs`,
          );
          if (!cancelled && messages.length > 0) {
            setHistoricalMessages((prev) => mergeHistorical(prev, messages));
            setHistoricalChannelLogged(true);
          }
        })
        .catch((e) => Logger.warn('[UserProfileCard] deep chat fetch failed:', e))
        .finally(() => {
          if (!cancelled) setDeepLoading(false);
        });

      await Promise.allSettled([fastP, deepP]);
      clearTimeout(failsafe);
    })();

    return () => {
      cancelled = true;
      clearTimeout(failsafe);
    };
  }, [showMessages, getChannelContext, username, userId]);

  // Keep the timeline live: while the messages view is open, Rust pushes every
  // new message from this user (user-history-message, shared across windows)
  // instead of the card polling the history service every 2.5 s.
  useEffect(() => {
    if (!showMessages || !userId) return;
    const key = historyKey(userId, provider);
    let cancelled = false;
    void invoke('watch_user_history', { userKey: key }).catch(() => {});
    const unlisten = listen<{ user_key: string; message: ParsedMessage }>('user-history-message', (event) => {
      if (cancelled || event.payload.user_key !== key) return;
      const msg = event.payload.message;
      if (!msg?.id) return;
      setLiveMessages((prev) => (prev.some((m) => m.id === msg.id) ? prev : prev.concat(msg)));
    });
    return () => {
      cancelled = true;
      unlisten.then((fn) => fn());
      void invoke('unwatch_user_history', { userKey: key }).catch(() => {});
    };
  }, [showMessages, userId, provider]);

  // Land on the newest message (chat reads newest-at-bottom) and stay pinned
  // there as live messages arrive — unless the reader scrolled up into older
  // history, tracked by stickBottomRef. Opening the messages view re-pins.
  useEffect(() => {
    if (!showMessages) return;
    stickBottomRef.current = true;
    const el = scrollBodyRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [showMessages]);

  useEffect(() => {
    if (!showMessages || !stickBottomRef.current) return;
    const el = scrollBodyRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [showMessages, historicalMessages, messageHistory, liveMessages]);

  // Compute selected paint from cached cosmetics (for instant display)
  const selectedPaint = useMemo(() => {
    // Prefer cached cosmetics for instant paint display
    const sources = [
      cachedProfile?.seventvCosmetics?.paints,
      profileData?.seventv_cosmetics?.paints,
      earlySeventv?.paints,
      wornCosmetics?.paints,
    ];
    for (const paints of sources) {
      const worn = (paints as any[] | undefined)?.find((p: any) => p.selected);
      if (worn) return worn;
    }
    return null;
  }, [cachedProfile?.seventvCosmetics, profileData?.seventv_cosmetics, earlySeventv, wornCosmetics]);

  const adjustNameColor = useNameColorAdjust();
  const shownColor = adjustNameColor(color) ?? color;
  const usernameStyle = useMemo(() =>
    selectedPaint ? computePaintStyle(selectedPaint as any, shownColor) : { color: shownColor },
    [selectedPaint, shownColor]
  );

  // Reactive caching for 7TV cosmetics displayed in profile
  useEffect(() => {
    // Cache 7TV badges
    const badges = cachedProfile?.seventvCosmetics?.badges || profileData?.seventv_cosmetics?.badges || [];
    badges.forEach((badge: any) => {
      if (badge?.id && !badge.localUrl) {
        const badgeUrl = `https://cdn.7tv.app/badge/${badge.id}/4x.webp`;
        queueCosmeticForCaching(badge.id, badgeUrl);
      }
    });
    
    // Cache paint image layers
    if (selectedPaint?.data?.layers) {
      selectedPaint.data.layers.forEach((layer: any) => {
        if (layer.ty?.__typename === 'PaintLayerTypeImage' && layer.ty.images) {
          const img = pickPaintLayerImage(layer.ty.images);
          if (img && !img.localUrl) {
            queueCosmeticForCaching(layer.id, img.url);
          }
        }
      });
    }
  }, [cachedProfile?.seventvCosmetics?.badges, profileData?.seventv_cosmetics?.badges, selectedPaint]);

  useEffect(() => {
    // cardWidth MUST match the `w-[402px]` className on the card root below
    // (matches DEFAULT_CHAT_WIDTH from App.tsx, same as the popout window).
    // When this literal drifts out of sync with the className the math places
    // the left edge based on the wrong width — too-large value pushes the
    // modal far to the left of the cursor (looks like "opens super far away"),
    // too-small overhangs the cursor on the right. Tailwind JIT requires the
    // literal in the class string so we can't extract a shared constant — keep
    // both in lockstep manually.
    const cardWidth = isMemberCard ? MEMBER_CARD_WIDTH : PROFILE_CARD_WIDTH, padding = 10, gap = 10;
    const estimatedHeight = showMessages || isMemberCard ? 700 : 400;
    let x = position.x - cardWidth - gap, y = position.y;
    if (x < padding) x = position.x + gap;
    if (y + estimatedHeight > window.innerHeight - padding) y = window.innerHeight - estimatedHeight - padding;
    y = Math.max(padding, y);
    setCardPosition({ x, y });
  }, [position, showMessages, isMemberCard]);

  const handleMouseDown = (e: React.MouseEvent<HTMLDivElement>) => {
    if (!(e.target as HTMLElement).closest('.profile-card-header')) return;
    setIsDragging(true);
    setDragOffset({ x: e.clientX - cardPosition.x, y: e.clientY - cardPosition.y });
  };

  useEffect(() => {
    const handleMouseMove = (e: MouseEvent) => {
      if (!isDragging) return;
      const cardWidth = cardRef.current?.offsetWidth || PROFILE_CARD_WIDTH;
      const cardHeight = cardRef.current?.offsetHeight || 400;
      const padding = 10;
      setCardPosition({
        x: Math.max(padding, Math.min(e.clientX - dragOffset.x, window.innerWidth - cardWidth - padding)),
        y: Math.max(padding, Math.min(e.clientY - dragOffset.y, window.innerHeight - cardHeight - padding))
      });
    };
    const handleMouseUp = () => setIsDragging(false);
    if (isDragging) {
      document.addEventListener('mousemove', handleMouseMove);
      document.addEventListener('mouseup', handleMouseUp);
    }
    return () => {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
    };
  }, [isDragging, dragOffset]);

  const cardStyle = useMemo(() => ({
    left: `${cardPosition.x}px`,
    top: `${cardPosition.y}px`,
    cursor: isDragging ? 'grabbing' : 'default'
  }), [cardPosition, isDragging]);

  const isStandaloneWindow = window.location.hash.startsWith('#/profile');

  // Handle follow/unfollow action via GQL mutations
  const handleFollowAction = useCallback(async () => {
    setFollowLoading(true);

    const action = isFollowing ? 'unfollow' : 'follow';
    Logger.debug(`[UserProfileCard] Initiating ${action} for ${username} (ID: ${userId})`);

    try {
      const command = isFollowing ? 'unfollow_channel' : 'follow_channel';
      await invoke(command, { targetUserId: userId });

      const nowFollowing = !isFollowing;
      setIsFollowing(nowFollowing);
      // Announce it. A chat composer blocked on "Follow to send a message"
      // listens for this and unblocks the moment you follow, rather than staying
      // locked until something happens to remount it.
      window.dispatchEvent(
        new CustomEvent('sn:follow-changed', {
          detail: { userId, following: nowFollowing },
        }),
      );
      Logger.debug(`[UserProfileCard] Successfully ${action}ed ${username}`);
    } catch (err: any) {
      Logger.error(`[UserProfileCard] ${action} error:`, err);
      useAppStore.getState().addToast(
        `Follow/Unfollow failed. Try logging out and back in via Settings to re-authenticate.`,
        'error'
      );
    } finally {
      setFollowLoading(false);
    }
  }, [userId, username, isFollowing]);

  // Categorized badges - prefer cached profile for instant display
  const { twitchBadges, seventvBadges, thirdPartyBadges, totalBadgeCount } = useMemo(() => {
    // Use cached profile for instant display, fallback to rust profile data
    const twitchFromCache = cachedProfile?.twitchBadges || [];
    const twitchFromRust = profileData?.badges?.display_badges || [];
    const earnedFromRust = profileData?.badges?.earned_badges || [];
    
    // Until a profile answers, show what the chat message itself carried:
    // the badges worn in this room, already resolved to images.
    const fromMessage =
      twitchFromCache.length + twitchFromRust.length + earnedFromRust.length === 0
        ? (messageBadges ?? [])
            .filter((b) => b.info)
            .map((b) => ({
              id: String(b.key).split('/')[0],
              image1x: b.info.image_url_1x,
              image2x: b.info.image_url_2x,
              image4x: b.info.image_url_4x,
              title: b.info.title,
              description: b.info.description,
            }))
        : [];

    // Merge display and earned badges for Twitch (deduped)
    const twitchMap = new Map<string, any>();
    [...twitchFromCache, ...twitchFromRust, ...earnedFromRust, ...fromMessage].forEach((b: any) => {
      if (!twitchMap.has(b.id || b.setID)) {
        twitchMap.set(b.id || b.setID, b);
      }
    });
    // Being this channel's broadcaster, mod, VIP or sub says nothing about who
    // someone is (everyone who goes live has the broadcaster badge), so the
    // card shows only badges that belong to the person.
    const twitchBadges = Array.from(twitchMap.values())
      .filter((b: any) => !isChannelScopedTwitchBadge(b.setID ?? String(b.id ?? '').split('/')[0]))
      .map((b: any) => ({
      id: b.id || `${b.setID}-${b.version}`,
      src: b.image4x || b.image_4x || b.image1x || b.image_1x,
      srcSet: `${b.image1x || b.image_1x} 1x, ${b.image2x || b.image_2x} 2x, ${b.image4x || b.image_4x} 4x`,
      title: b.title,
      description: b.description
    }));

    // 7TV badges - prefer cached
    const seventvFromCache = cachedProfile?.seventvCosmetics?.badges || [];
    const seventvFromRust = profileData?.seventv_cosmetics?.badges || earlySeventv?.badges || [];
    const seventvFromChat =
      seventvFromCache.length + seventvFromRust.length === 0 ? wornCosmetics?.badges ?? [] : [];
    const seventvMap = new Map<string, any>();
    [...seventvFromCache, ...seventvFromRust, ...seventvFromChat].forEach((b: any) => {
      if (!seventvMap.has(b.id)) {
        seventvMap.set(b.id, b);
      }
    });
    const seventvBadges = Array.from(seventvMap.values()).map((b: any) => {
      const urls = getBadgeImageUrls(b);
      return {
        id: b.id,
        src: urls.url4x || `https://cdn.7tv.app/badge/${b.id}/4x.webp`,
        fallbackUrls: getBadgeFallbackUrls(b.id).slice(1),
        srcSet: urls.url1x ? `${urls.url1x} 1x, ${urls.url2x} 2x, ${urls.url4x} 4x` : undefined,
        title: b.tooltip || b.description || b.name,
        name: b.name
      };
    });

    // Third-party badges (FFZ, Chatterino, Homies) - prefer cached
    const thirdPartyFromCache = cachedProfile?.thirdPartyBadges || [];
    const thirdPartyFromRust = profileData?.badges?.third_party_badges || [];
    const thirdPartyMap = new Map<string, any>();
    [...thirdPartyFromCache, ...thirdPartyFromRust].forEach((b: any) => {
      if (!thirdPartyMap.has(b.id)) {
        thirdPartyMap.set(b.id, b);
      }
    });
    const thirdPartyBadges = Array.from(thirdPartyMap.values()).map((b: any) => ({
      id: b.id,
      src: b.image4x || b.imageUrl,
      srcSet: b.image1x && b.image2x && b.image4x
        ? `${b.image1x} 1x, ${b.image2x} 2x, ${b.image4x} 4x`
        : undefined,
      title: b.title,
      provider: b.provider
    }));

    // BTTV Pro loyalty badge (resolved over the BTTV socket, not the cached
    // contributor feed). Tagged provider 'BTTV' so it lands in the BetterTTV
    // group via THIRD_PARTY_PROVIDER_GROUPS, alongside any contributor badge.
    if (bttvProBadge?.url) {
      thirdPartyBadges.push({
        id: 'bttv-pro',
        src: bttvProBadge.url,
        srcSet: undefined,
        title: 'BTTV Pro',
        provider: 'BTTV',
      });
    }

    return {
      twitchBadges,
      seventvBadges,
      thirdPartyBadges,
      totalBadgeCount: twitchBadges.length + seventvBadges.length + thirdPartyBadges.length
    };
  }, [cachedProfile, profileData, bttvProBadge, earlySeventv, wornCosmetics, messageBadges]);

  // StreamNook identity loadout for the viewed user. Drives which third-party
  // badges other members see: once a member curates (customized), only the
  // badges they opted in show; an un-curated member still shows everything (the
  // default). Synchronous cache peek + version subscription so the card repaints
  // when the loadout lands or the member edits it; the effect kicks the fetch.
  useSyncExternalStore(subscribeIdentityVersion, getIdentityVersion, getIdentityVersion);
  const identityLoadout = userId ? getIdentityFromMemoryCache(userId) : null;
  useEffect(() => {
    if (userId) void getIdentityWithCache(userId).catch(() => {});
  }, [userId]);
  const visibleThirdPartyBadges = useMemo(() => {
    if (!identityLoadout?.customized) return thirdPartyBadges;
    // Render in the member's CHOSEN order (their stored loadout key order) so the
    // profile card matches chat exactly. Chat resolves these through the server's
    // resolveLoadout, which preserves the saved `badges` array order; filtering
    // the provider-ordered thirdPartyBadges list (the previous approach) kept the
    // same set but a different order, which is the drift between chat and profile.
    const byKey = new Map<string, (typeof thirdPartyBadges)[number]>(
      thirdPartyBadges.map((b) => [`${b.provider}:${b.id}`, b]),
    );
    return identityLoadout.badges
      .map((k) => byKey.get(k))
      .filter((b): b is NonNullable<typeof b> => b != null);
  }, [thirdPartyBadges, identityLoadout?.customized, identityLoadout?.badges]);
  // Count what's actually shown (curation can hide some third-party badges) so
  // the "Badges N" header doesn't overcount hidden ones.
  const visibleTotalBadgeCount =
    totalBadgeCount - thirdPartyBadges.length + visibleThirdPartyBadges.length;

  const twitchProfile = profileData?.twitch_profile ?? earlyTwitch ?? undefined;
  const ivrData = profileData?.ivr_data;

  // The @handle must be the real LOGIN, not the display name. The login/display
  // name passed in from chat can be identical (the parser sources both from the
  // display-name tag), so once Helix resolves the authoritative login by user-id
  // we prefer it for the handle and every Twitch-facing action (links, mod
  // commands, whisper). Falls back to the passed value while the profile loads.
  const login = twitchProfile?.login || username;

  // Prefer the user's 7TV animated avatar when they have one; otherwise the
  // platform's own profile image. 7TV exposes no banner, only this avatar.
  //
  // `twitchProfile` is null on a provider card by design (the Helix fetch is
  // skipped), so reading only from it left every Kick and YouTube card with a
  // blank avatar and banner even though both are fetched and parsed.
  const avatarUrl =
    profileData?.seventv_cosmetics?.avatar_url ||
    twitchProfile?.profile_image_url ||
    kickProfile?.avatar_url ||
    youtubeProfile?.avatar_url ||
    undefined;

  const bannerStyle = useMemo(() => {
    const bannerUrl =
      twitchProfile?.banner_image_url ||
      twitchProfile?.offline_image_url ||
      kickProfile?.banner_url ||
      youtubeProfile?.banner_url;
    if (bannerUrl) {
      return {
        backgroundImage: `url(${bannerUrl})`,
        backgroundSize: 'cover',
        backgroundPosition: 'center',
      } as const;
    }
    const accent = color || '#9146FF';
    return {
      backgroundImage: `linear-gradient(135deg, ${accent}40 0%, ${accent}10 50%, color-mix(in srgb, var(--color-accent) 13%, transparent) 100%)`,
    } as const;
  }, [twitchProfile?.banner_image_url, twitchProfile?.offline_image_url, kickProfile?.banner_url, youtubeProfile?.banner_url, color]);

  const handleWhisper = useCallback(async () => {
    const user = {
      id: userId,
      login,
      display_name: displayName,
      profile_image_url: avatarUrl,
    };
    if (onStartWhisper) {
      onStartWhisper(user);
    } else if (isStandaloneWindow) {
      try {
        // Ensure the main window is back + listening (going live may have closed
        // it) before handing off the whisper, then close this profile card.
        const { ensureMainAndEmit } = await import('../utils/ensureMainWindow');
        await ensureMainAndEmit('start-whisper', user);
        const { getCurrentWindow } = await import('@tauri-apps/api/window');
        await getCurrentWindow().close();
      } catch (err) {
        Logger.error('Failed to emit whisper event:', err);
      }
    } else {
      useAppStore.getState().openWhisperWithUser(user);
    }
    onClose();
  }, [userId, login, displayName, avatarUrl, onStartWhisper, isStandaloneWindow, onClose]);

  // On a member card each provider group is its own glass tile inside the
  // badges section, the same two-tier finish as the overview's sections.
  // Tiles flow side by side, so a client with one badge doesn't claim a whole
  // row; a big group simply fills its own.
  const badgeGroupTile = isMemberCard ? 'glass-tile p-2.5 flex-auto min-w-[120px]' : '';
  const memberSectionStyle =
    isMemberCard && memberView.themeRgb ? { borderColor: `rgba(${memberView.themeRgb}, 0.3)` } : undefined;

  const renderBadgeGroup = (
    label: string,
    badges: any[],
    renderItem: (badge: any, index: number) => React.ReactNode,
    groupKey?: string,
  ) => {
    if (badges.length === 0) return null;
    return (
      <div key={groupKey} className={badgeGroupTile}>
        <p className="text-[10px] text-textSecondary uppercase tracking-wider font-medium mb-2">
          {label} <span className="text-textSecondary/50 tabular-nums">{badges.length}</span>
        </p>
        <div className="flex items-center gap-1.5 flex-wrap">
          {badges.map(renderItem)}
        </div>
      </div>
    );
  };

  // Inline identity chips (Twitch ✓ → 7TV paint → StreamNook #N). Shared by the
  // full identity block in the body and the compact single-line row that fades
  // into the collapsed header, so the two never drift apart.
  const identityChips = (
    <>
      {twitchProfile?.broadcaster_type === 'partner' && (
        <Tooltip content="Verified Partner" side="top">
          <span className="inline-flex flex-shrink-0">
            <svg className="w-[18px] h-[18px]" viewBox="0 0 16 16" fill="#9146FF">
              <path fillRule="evenodd" d="M12.5 3.5 8 2 3.5 3.5 2 8l1.5 4.5L8 14l4.5-1.5L14 8l-1.5-4.5ZM7 11l4.5-4.5L10 5 7 8 5.5 6.5 4 8l3 3Z" clipRule="evenodd" />
            </svg>
          </span>
        </Tooltip>
      )}
      {selectedPaint && <PaintChip paint={selectedPaint as never} color={color} />}
      {seventvUserId && cardPrefs?.show_seventv_link !== false && <SevenTvProfileButton seventvUserId={seventvUserId} />}
    </>
  );

  // A member card's actions: one compact row right under the hero, so the
  // StreamNook profile below it stays the first thing you read.
  const memberActionBtn =
    'glass-button flex flex-1 items-center justify-center gap-1.5 rounded-md px-2 py-2 text-xs text-white transition-colors hover:bg-accent/20';
  const memberActionBar = (
    <div className="flex items-stretch gap-1.5">
      <Tooltip content={isFollowing ? `Unfollow ${displayName}` : `Follow ${displayName}`} side="top">
        <button
          onClick={handleFollowAction}
          disabled={followLoading}
          className={`${memberActionBtn} ${followLoading ? 'cursor-wait opacity-50' : ''}`}
        >
          {followLoading ? (
            <Loader2 size={14} className="animate-spin text-accent" />
          ) : isFollowing ? (
            <UserMinus size={14} className="text-error" />
          ) : (
            <UserPlus size={14} className="text-success" />
          )}
          {isFollowing ? 'Unfollow' : 'Follow'}
        </button>
      </Tooltip>
      <button onClick={handleWhisper} className={memberActionBtn}>
        <MessageCircle size={14} className="text-accent" />
        Whisper
      </button>
      <button onClick={() => setShowMessages(true)} className={memberActionBtn}>
        <History size={14} className="text-accent" />
        Messages
      </button>
      <button
        onClick={() => {
          useAppStore.getState().startOfflineChat(login);
          onClose();
        }}
        className={memberActionBtn}
      >
        Join chat
      </button>
      <a href={`https://www.twitch.tv/${login}`} target="_blank" rel="noopener noreferrer" className={memberActionBtn}>
        Twitch
      </a>
      <button
        onClick={async () => {
          try {
            await navigator.clipboard.writeText(buildShareUrl(login));
            setShareCopied(true);
            window.setTimeout(() => setShareCopied(false), 1400);
          } catch (err) {
            Logger.error('[UserProfileCard] Failed to copy share link:', err);
          }
        }}
        className={`${memberActionBtn} ${shareCopied ? 'text-success' : ''}`}
      >
        {shareCopied ? <Check size={14} className="text-success" /> : <Share2 size={14} className="text-accent" />}
        {shareCopied ? 'Copied' : 'Share'}
      </button>
    </div>
  );
  // Section headings on a member card, matching the overview's own.
  const memberHeading = (label: string, count?: number) => (
    <h4 className="mb-2.5 flex items-center gap-1.5 text-[12px] font-semibold uppercase tracking-[0.14em] text-textPrimary">
      {label}
      {count !== undefined && <span className="font-medium tabular-nums text-textSecondary/60">{count}</span>}
    </h4>
  );

  // Every badge the person has, grouped by provider. A member card shows it
  // near the top, under their relics; the plain card further down.
  const badgesPanel = (
    <>
            {/* Badges, provider-grouped inside one panel, no inner scroll.
                Section also renders when the user has the StreamNook badge but
                no chat badges, so the StreamNook identity always surfaces.
                Order matches chat and the member hero (utils/badgeOrder):
                Twitch, 7TV, third-party, then StreamNook. */}
            {visibleTotalBadgeCount === 0 && streamNookBadgeCount === 0 && profileLoading && provider === 'twitch' && (
              <div aria-hidden="true">
                <p className="text-[11px] text-textSecondary uppercase tracking-wider font-semibold mb-2">Badges</p>
                <div className="glass-panel rounded-md px-3 py-3 flex items-center gap-1.5">
                  {[0, 1, 2, 3, 4].map((i) => (
                    <span key={i} className="w-5 h-5 rounded bg-white/[0.06] animate-pulse" />
                  ))}
                </div>
              </div>
            )}
            {(visibleTotalBadgeCount > 0 || streamNookBadgeCount > 0) && (
              <div className={isMemberCard ? 'settings-card p-3.5' : ''} style={memberSectionStyle}>
                {isMemberCard ? (
                  memberHeading('Badges', visibleTotalBadgeCount + streamNookBadgeCount)
                ) : (
                  <p className="text-[11px] text-textSecondary uppercase tracking-wider font-semibold mb-2">
                    Badges <span className="text-textSecondary/60 tabular-nums">{visibleTotalBadgeCount + streamNookBadgeCount}</span>
                  </p>
                )}
                <div className={isMemberCard ? 'flex flex-wrap gap-2' : 'glass-panel rounded-md px-3 py-3 space-y-3'}>
                {renderBadgeGroup('Twitch', twitchBadges, (b, i) => (
                  <Tooltip key={`twitch-${b.id}-${i}`} content={b.description ? `${b.title}\n${b.description}` : b.title} side="top">
                    <img
                      src={b.src}
                      alt={b.title}
                      className="w-5 h-5 cursor-pointer hover:scale-110 transition-transform"
                      onClick={() => openBadgesWithTargetInMain({ tab: 'twitch-badges', query: b.title })}
                      onError={e => { e.currentTarget.style.display = 'none'; }}
                    />
                  </Tooltip>
                ))}
                {renderBadgeGroup('7TV', seventvBadges, (b, i) => (
                  <Tooltip key={`7tv-${b.id}-${i}`} content={b.title} side="top">
                    <FallbackImage
                      src={b.src}
                      fallbackUrls={b.fallbackUrls}
                      alt={b.title}
                      className="w-5 h-5 cursor-pointer hover:scale-110 transition-transform"
                      onClick={() => openBadgesWithBadgeInMain(b.id)}
                    />
                  </Tooltip>
                ))}
                {(() => {
                  // Third-party chat-client badges, grouped per provider so each
                  // badge shows its source (FrankerFaceZ / BetterTTV / Chatterino
                  // / …) instead of a single undifferentiated "Other" row.
                  const renderThirdPartyItem = (b: any, i: number) => (
                    <Tooltip key={`3p-${b.id}-${i}`} content={b.title} side="top">
                      <img
                        src={b.src}
                        alt={b.title}
                        className="w-5 h-5 cursor-pointer hover:scale-110 transition-transform"
                        onClick={() => openBadgesWithTargetInMain(
                          // Case-insensitive for the same reason the groups are:
                          // contributor badges come from Rust as 'bttv', while the
                          // Pro badge is tagged 'BTTV' to match its loadout key.
                          providerGroupKey(b.provider) === 'bttv'
                            ? { tab: 'bttv', query: b.title }
                            : { tab: 'chat-clients', query: b.title },
                        )}
                        onError={e => { e.currentTarget.style.display = 'none'; }}
                      />
                    </Tooltip>
                  );
                  const known = new Set(THIRD_PARTY_PROVIDER_GROUPS.map(g => g.key));
                  const ungrouped = visibleThirdPartyBadges.filter(b => !known.has(providerGroupKey(b.provider)));
                  return (
                    <>
                      {THIRD_PARTY_PROVIDER_GROUPS.map(({ key, label }) =>
                        renderBadgeGroup(
                          label,
                          visibleThirdPartyBadges.filter(b => providerGroupKey(b.provider) === key),
                          renderThirdPartyItem,
                          `3p-${key}`,
                        ),
                      )}
                      {renderBadgeGroup('Other', ungrouped, renderThirdPartyItem, '3p-other')}
                    </>
                  );
                })()}
                {/* StreamNook badges. The active selection renders with the full
                    tier-card tooltip (user identity surface). Other owned-but-
                    inactive badges render as simple thumbnails alongside it,
                    matching how 7TV / Third-party expose multiple owned items. */}
                {streamNookBadgeCount > 0 && (
                  <div className={badgeGroupTile}>
                    <p className="text-[10px] text-textSecondary uppercase tracking-wider font-medium mb-2">
                      StreamNook <span className="text-textSecondary/50 tabular-nums">{streamNookBadgeCount}</span>
                    </p>
                    <div className="flex items-center gap-1.5 flex-wrap">
                      {streamNookUserNumber !== null && (
                        <StreamNookBadge userId={cardMemberId} />
                      )}
                      {inactiveOwnedSlugs.map(({ slug, cosmetic, asset }) => (
                        <Tooltip key={`sn-${slug}`} content={cosmetic.name} side="top">
                          <img
                            src={asset}
                            alt={cosmetic.name}
                            className="w-6 h-6 inline-block object-contain cursor-pointer hover:scale-110 transition-transform"
                            draggable={false}
                            onClick={openBadgesOnStreamNookInMain}
                          />
                        </Tooltip>
                      ))}
                    </div>
                  </div>
                )}
              </div>
            </div>
          )}
    </>
  );

  // Follow, whisper, share, and the recent-messages toggle.
  const actionsBlock = (
    <>
            {/* Actions: 2x2 grid + messages toggle */}
            <div className="space-y-2">
            <div className="grid grid-cols-2 gap-2">
              <Tooltip content={followLoading ? 'Processing...' : isFollowing ? `Unfollow ${displayName}` : `Follow ${displayName}`} side="top">
                <button
                  onClick={handleFollowAction}
                  disabled={followLoading}
                  className={`glass-button text-white text-xs py-2.5 px-3 rounded-md text-center transition-colors flex items-center justify-center gap-1.5 w-full ${followLoading
                    ? 'opacity-50 cursor-wait'
                    : isFollowing
                      ? 'hover:bg-error/20 border-red-500/30'
                      : 'hover:bg-success/20 border-green-500/30'
                    }`}
                >
                  {followLoading ? (
                    <>
                      <Loader2 size={14} className="animate-spin text-accent" />
                      <span>Working...</span>
                    </>
                  ) : isFollowing ? (
                    <>
                      <UserMinus size={14} className="text-error" />
                      <span>Unfollow</span>
                    </>
                  ) : (
                    <>
                      <UserPlus size={14} className="text-success" />
                      <span>Follow</span>
                    </>
                  )}
                </button>
              </Tooltip>
              <button
                onClick={handleWhisper}
                className="glass-button text-white text-xs py-2.5 px-3 rounded-md text-center hover:bg-accent/20 transition-colors flex items-center justify-center gap-1.5 w-full"
              >
                <MessageCircle size={14} className="text-accent" />
                Whisper
              </button>
              <button
                onClick={() => {
                  useAppStore.getState().startOfflineChat(login);
                  onClose();
                }}
                className="glass-button text-white text-xs py-2.5 px-3 rounded-md text-center hover:bg-accent/20 transition-colors w-full"
              >
                Join Chat
              </button>
              <a
                href={`https://www.twitch.tv/${login}`}
                target="_blank"
                rel="noopener noreferrer"
                className="glass-button text-white text-xs py-2.5 px-3 rounded-md text-center hover:bg-accent/20 transition-colors flex items-center justify-center w-full"
              >
                Open on Twitch
              </a>
              {/* Share. Copies the streamnook.app/w/<channel> link; icon and label
                  pop to a check on copy. Full width under the action grid. */}
              <button
                onClick={async () => {
                  try {
                    await navigator.clipboard.writeText(buildShareUrl(login));
                    setShareCopied(true);
                    window.setTimeout(() => setShareCopied(false), 1400);
                  } catch (err) {
                    Logger.error('[UserProfileCard] Failed to copy share link:', err);
                  }
                }}
                className={`col-span-2 glass-button text-xs py-2.5 px-3 rounded-md text-center transition-colors flex items-center justify-center gap-1.5 w-full ${shareCopied ? 'text-success' : 'text-white hover:bg-accent/20'}`}
              >
                <span key={shareCopied ? 'copied' : 'share'} className="inline-flex animate-in zoom-in-50 duration-200">
                  {shareCopied ? <Check size={14} className="text-success" /> : <Share2 size={14} className="text-accent" />}
                </span>
                {shareCopied ? 'Link copied!' : 'Share'}
              </button>
            </div>

            {/* Trigger for the messages takeover. Chevron points UP because
                clicking expands the messages region upward into the body
                above. No count badge — the local in-session buffer is just
                one of three sources we pull from, so showing its count would
                be misleading (often 0 even when Justlog / robotty have
                plenty of history). */}
            <button
              onClick={() => setShowMessages(true)}
              className="w-full glass-button text-white text-xs py-2 px-3 rounded-md text-center transition-colors flex items-center justify-center gap-1.5 hover:bg-accent/15"
            >
              <ChevronUp size={14} />
              Show recent messages
            </button>
          </div>
    </>
  );

  return (
    <>
      {!isStandaloneWindow && (
        <div className="fixed inset-0 z-40 group">
          <div className="absolute inset-0 group-hover:pointer-events-none" onClick={onClose} />
        </div>
      )}
      <div
        ref={cardRef}
        className={`${isStandaloneWindow ? 'w-full h-full' : `fixed z-50 ${isMemberCard ? 'w-[760px]' : 'w-[440px]'} max-h-[88vh]`} sn-light-off user-profile-card backdrop-blur-xl shadow-2xl border border-borderSubtle rounded-lg overflow-hidden flex flex-col`}
        style={isStandaloneWindow ? { backgroundColor: 'rgba(0, 0, 0, 0.75)' } : cardStyle}
        onMouseDown={isStandaloneWindow ? undefined : handleMouseDown}
      >
        {isMemberCard && <MemberProfileBackdrop view={memberView} />}
        {isMemberCard && cardMemberId ? (
          <div className="relative z-10 profile-card-header cursor-grab active:cursor-grabbing flex-shrink-0">
            <MemberProfileHero userId={cardMemberId} view={memberView} preview={false} close={onClose} />
          </div>
        ) : (
        <>
        {/* Sticky header: banner + floating avatar (absolute, so it can overflow the banner without getting clipped by the scroll body below). */}
        <div className="relative z-10 profile-card-header cursor-grab active:cursor-grabbing flex-shrink-0">
          <motion.div className="relative overflow-hidden" style={{ height: prefersReducedMotion ? 96 : bannerHeight }}>
            <motion.div
              className="absolute inset-0"
              style={prefersReducedMotion ? bannerStyle : { ...bannerStyle, scale: bannerZoom, filter: bannerBlur }}
            />
            {!prefersReducedMotion && (
              <motion.div className="absolute inset-0" style={{ backgroundColor: '#000', opacity: bannerScrim }} />
            )}
          </motion.div>
          <motion.div
            className="absolute -bottom-10 left-4 w-20 h-20 rounded-full border-2 border-secondary bg-secondary overflow-hidden shadow-lg"
            style={prefersReducedMotion ? undefined : { scale: avatarScale, y: avatarLift, transformOrigin: 'left bottom' }}
          >
            {avatarUrl ? (
              <img
                src={avatarUrl}
                alt={displayName}
                className={`w-full h-full object-cover ${isBanned ? 'grayscale' : ''}`}
              />
            ) : profileLoading ? (
              <div className="w-full h-full bg-white/[0.06] animate-pulse" />
            ) : (
              <div className="w-full h-full flex items-center justify-center bg-accent/20">
                <span className="text-2xl font-bold text-textPrimary">{displayName[0].toUpperCase()}</span>
              </div>
            )}
            {isBanned && (
              <div className="absolute inset-0 flex items-end justify-center bg-black/45">
                <span className="mb-1 rounded px-1.5 py-px bg-error/90 text-[9px] font-bold tracking-wider text-white">
                  BANNED
                </span>
              </div>
            )}
          </motion.div>
          {/* Compact name + chips, vertically centered next to the shrunken
              avatar. Hidden in the expanded state, fades in as the header
              collapses so the identity stays visible once the body's full name
              has scrolled away. */}
          {!prefersReducedMotion && (
            <motion.div
              className="absolute left-[72px] right-12 top-0 bottom-0 flex items-center gap-2 overflow-hidden"
              style={{ opacity: collapsedNameOpacity, pointerEvents: collapsedNamePointer }}
            >
              <span className="text-sm font-bold leading-tight truncate min-w-0" style={usernameStyle}>{displayName}</span>
              {pronouns && (
                <span className="flex-shrink-0 rounded-full border border-white/10 bg-white/5 px-1.5 py-[1px] text-[10px] font-medium leading-none text-textSecondary">
                  {pronouns}
                </span>
              )}
              <div className="flex items-center gap-1.5 flex-shrink-0">{identityChips}</div>
            </motion.div>
          )}
          <button
            onClick={onClose}
            className="absolute top-2.5 right-2.5 z-10 p-1.5 glass-button text-textSecondary hover:text-textPrimary rounded-full"
            aria-label="Close profile"
          >
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        </>
        )}

        {/* Single scroll body. One padded container, vertical rhythm via space-y, no section dividers. */}
        <div
          ref={scrollBodyRef}
          className={`relative z-[1] flex-1 overflow-y-auto min-h-0 scrollbar-thin ${isMemberCard && memberView.hasAtmosphere ? 'bg-[rgba(10,10,14,0.58)] backdrop-blur-[44px]' : ''}`}
          style={isMemberCard && memberView.hasAtmosphere ? ({ '--glass-strength': '0.45' } as React.CSSProperties) : undefined}
          onScroll={(e) => {
            const el = e.currentTarget;
            // Re-pin only when parked within a line or two of the bottom, so
            // scrolling up to read older history stops the live auto-scroll.
            stickBottomRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 60;
          }}
        >
          <motion.div className="px-4 pb-4 space-y-4" style={{ paddingTop: isMemberCard ? 14 : prefersReducedMotion ? 48 : bodyPadTop }}>
            {/* Takeover animation. When showMessages flips true, the profile
                body (identity + stats + badges + actions) slides UP and out and
                the messages view slides in — a "curtain reveal" feel.
                AnimatePresence with `mode="wait"` ensures the exit completes
                before the enter starts. The wrapper has `overflow-hidden` so the
                slide stays clipped to the body instead of leaking under the
                header, where the collapsed compact name carries the identity. */}
            <div className="relative overflow-hidden">
              <AnimatePresence mode="wait" initial={false}>
                {showMessages ? (
                  <motion.div
                    key="messages-view"
                    initial={{ y: -16, opacity: 0 }}
                    animate={{ y: 0, opacity: 1 }}
                    exit={{ y: -16, opacity: 0 }}
                    transition={{ type: 'tween', duration: 0.22, ease: 'easeOut' }}
                    className="-mx-2"
                  >
                    {/* Sticky so "Back to profile" stays reachable once the
                        timeline auto-scrolls to the newest message. */}
                    <div className="sticky top-0 z-20 flex items-center justify-between px-2 py-2 -mt-2 bg-secondary/95 backdrop-blur-sm">
                      <button
                        type="button"
                        onClick={() => setShowMessages(false)}
                        className="inline-flex items-center gap-1 text-[11px] text-textSecondary hover:text-textPrimary transition-colors"
                      >
                        <ChevronDown size={14} />
                        Back to profile
                      </button>
                      <span className="text-[10px] text-textSecondary uppercase tracking-wider font-semibold">
                        Chat history{(historicalLoading || deepLoading) ? ' · loading…' : ''}
                      </span>
                    </div>
                    {(() => {
                      // Unified message timeline. Historical (Twitch GQL +
                      // Justlog + Robotty, merged in Rust) AND in-session
                      // sources normalize to { ts, content, id }, dedupe by
                      // Twitch IRC message UUID across BOTH layers, then
                      // sort + group by day. Same timeline shape 7TV's
                      // extension uses.
                      const readTag = (msg: any, key: string): string | undefined => {
                        const tags = msg?.tags;
                        if (!tags) return undefined;
                        if (tags instanceof Map) return tags.get(key);
                        if (typeof tags === 'object') return tags[key];
                        return undefined;
                      };
                      const sessionTimestamp = (msg: any): number | null => {
                        // Rust UserMessageSummary path: top-level unix-ms string.
                        if (typeof msg?.timestamp === 'string') {
                          const n = parseInt(msg.timestamp, 10);
                          if (Number.isFinite(n)) return n;
                        }
                        // Frontend ParsedMessage path: from IRC tags.
                        const raw = readTag(msg, 'tmi-sent-ts');
                        const n = raw ? parseInt(raw, 10) : NaN;
                        return Number.isFinite(n) ? n : null;
                      };
                      const historicalTs = (iso: string): number | null => {
                        const n = Date.parse(iso);
                        return Number.isFinite(n) ? n : null;
                      };

                      type TimelineEntry = { ts: number; content: string; id?: string };
                      const rawEntries: TimelineEntry[] = [];
                      for (const m of historicalMessages) {
                        const ts = historicalTs(m.timestamp);
                        if (ts !== null) rawEntries.push({ ts, content: m.content, id: m.id });
                      }
                      for (const m of messageHistory) {
                        const ts = sessionTimestamp(m) ?? Date.now();
                        // IRC `id` tag is the Twitch message UUID. Prefer the
                        // top-level field (UserMessageSummary path) and fall
                        // back to the tags map (ParsedMessage path). Same id
                        // each historical source carries, so it's the right
                        // cross-layer dedupe key.
                        const id = (m as any).id ?? readTag(m, 'id');
                        rawEntries.push({ ts, content: m.content, id });
                      }
                      // Live messages polled while the card is open. Same
                      // unix-ms shape as messageHistory; the id dedupe below
                      // collapses overlap with the initial snapshot.
                      for (const m of liveMessages) {
                        const ts = sessionTimestamp(m) ?? Date.now();
                        const id = (m as any).id ?? readTag(m, 'id');
                        rawEntries.push({ ts, content: m.content, id });
                      }

                      // Dedupe by id when present; fall back to
                      // (content, ts within 2s) for the rare case a source
                      // didn't carry an id. The in-session buffer for a busy
                      // chatter can easily overlap with Robotty's last-100
                      // or MessageBufferChatHistory's ~30; without this pass
                      // the same message appears twice in the popup.
                      const seenIds = new Set<string>();
                      const entries: TimelineEntry[] = [];
                      for (const e of rawEntries) {
                        if (e.id) {
                          if (seenIds.has(e.id)) continue;
                          seenIds.add(e.id);
                        } else {
                          const dup = entries.find(
                            (x) => x.content === e.content && Math.abs(x.ts - e.ts) <= 2000,
                          );
                          if (dup) continue;
                        }
                        entries.push(e);
                      }
                      entries.sort((a, b) => a.ts - b.ts);

                      if (entries.length === 0 && !historicalLoading && !deepLoading) {
                        // Say WHY the panel is empty. Twitch serves its own
                        // multi-year archive only to moderators of the channel,
                        // so an empty card is usually a permission boundary
                        // rather than a failure, and silently showing nothing
                        // made those two look identical.
                        let reason: string;
                        if (modLogsStatus === 'no_access') {
                          reason = "Twitch only shares a viewer's past messages with moderators of this channel.";
                        } else if (modLogsStatus === 'auth_required') {
                          reason = 'Reconnect your Twitch account to load messages from Twitch history.';
                        } else if (modLogsStatus === 'failed') {
                          reason = 'Could not reach Twitch for older messages.';
                        } else if (historicalChannelLogged) {
                          reason = 'New messages from this user will appear here.';
                        } else {
                          reason = 'No historical messages available for this channel.';
                        }
                        return (
                          <div className="px-2">
                            <div className="flex flex-col items-center justify-center py-12 text-center">
                              <MessageCircle size={32} className="text-textSecondary/30 mb-2" />
                              <p className="text-sm text-textSecondary">No messages found</p>
                              <p className="text-xs text-textSecondary/60 mt-1">{reason}</p>
                            </div>
                          </div>
                        );
                      }

                      // Group entries by calendar day for the timeline separators.
                      const dayKey = (ts: number) => {
                        const d = new Date(ts);
                        return `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;
                      };
                      const groups: { key: string; label: string; entries: TimelineEntry[] }[] = [];
                      const todayKey = dayKey(Date.now());
                      const yesterdayKey = dayKey(Date.now() - 86_400_000);
                      const dayLabel = (ts: number, key: string): string => {
                        if (key === todayKey) return 'Today';
                        if (key === yesterdayKey) return 'Yesterday';
                        return new Date(ts).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });
                      };
                      let currentKey = '';
                      for (const e of entries) {
                        const k = dayKey(e.ts);
                        if (k !== currentKey) {
                          groups.push({ key: k, label: dayLabel(e.ts, k), entries: [] });
                          currentKey = k;
                        }
                        groups[groups.length - 1].entries.push(e);
                      }

                      const formatTime = (ts: number) => {
                        const d = new Date(ts);
                        const h = d.getHours().toString().padStart(2, '0');
                        const m = d.getMinutes().toString().padStart(2, '0');
                        return `${h}:${m}`;
                      };

                      return (
                        <div className="px-2">
                          {groups.map((g) => (
                            <div key={g.key} className="mb-3 last:mb-0">
                              <div className="flex items-center gap-2 my-2">
                                <div className="flex-1 h-px bg-borderSubtle" />
                                <span className="text-[10px] text-textSecondary/70 uppercase tracking-wider tabular-nums">
                                  {g.label}
                                </span>
                                <div className="flex-1 h-px bg-borderSubtle" />
                              </div>
                              <div className="space-y-0.5">
                                {g.entries.map((e, idx) => (
                                  <div
                                    key={`${g.key}-${idx}`}
                                    className="flex items-baseline gap-2 py-1 px-2 rounded hover:bg-white/[0.03] transition-colors"
                                  >
                                    <span className="text-[10px] text-textSecondary/60 tabular-nums shrink-0 mt-px">
                                      {formatTime(e.ts)}
                                    </span>
                                    <span className="text-[13px] font-semibold shrink-0" style={{ color }}>
                                      {displayName}:
                                    </span>
                                    <span className="text-[13px] text-textPrimary break-words leading-snug min-w-0">
                                      <EmoteText
                                        text={e.content}
                                        emoteMap={emoteNameMap}
                                        keyPrefix={`hist-${g.key}-${idx}`}
                                        imgClassName="inline-block align-middle object-contain h-[18px] max-h-[18px]"
                                      />
                                    </span>
                                  </div>
                                ))}
                              </div>
                            </div>
                          ))}
                        </div>
                      );
                    })()}
                  </motion.div>
                ) : (
                  <motion.div
                    key="profile-body"
                    initial={{ y: -16, opacity: 0 }}
                    animate={{ y: 0, opacity: 1 }}
                    exit={{ y: -16, opacity: 0 }}
                    transition={{ type: 'tween', duration: 0.22, ease: 'easeOut' }}
                    className="space-y-4"
                  >
                    {isMemberCard && cardMemberId && (
                      <>
                        {memberActionBar}
                        <MemberProfileSections userId={cardMemberId} view={memberView} afterRelics={badgesPanel} />
                        {memberHeading('On Twitch')}
                      </>
                    )}
                    {/* Identity leads the profile body so it animates in/out as
                        one unit with the rest; in the messages view the collapsed
                        header's compact name carries the identity instead. */}
            <div>
              {!isMemberCard && (
                <>
                  <div className="flex items-center gap-2 flex-wrap">
                    <h3 className="text-xl font-bold leading-tight" style={usernameStyle}>{displayName}</h3>
                    {identityChips}
                  </div>
                  <p className="text-sm text-textSecondary mt-0.5">
                    {provider === 'youtube' ? (youtubeProfile?.handle ?? `@${login}`) : `@${login}`}
                  </p>
                </>
              )}
              {twitchProfile?.description && (
                <p className="text-sm text-textPrimary/85 mt-2.5 leading-relaxed">{twitchProfile.description}</p>
              )}
              {provider === 'kick' && kickProfile?.bio && (
                <p className="text-sm text-textPrimary/85 mt-2.5 leading-relaxed">{kickProfile.bio}</p>
              )}
              {provider === 'youtube' && youtubeProfile?.description && (
                <p className="text-sm text-textPrimary/85 mt-2.5 leading-relaxed line-clamp-4">
                  {youtubeProfile.description}
                </p>
              )}
            </div>

            {/* Platform stats. Deliberately its own block rather than branches
                threaded through the Twitch panel below: that panel is built on
                Helix + IVR shapes, and every row in it would need a null-guard for
                data these platforms simply do not have. Twitch-only rows (Chatters,
                Last live, whispers) are HIDDEN here rather than repurposed —
                better no row than a wrong one. */}
            {provider === 'kick' && kickProfile && (
              <div className="glass-panel rounded-md px-3 py-2 space-y-1">
                <ProviderStatRow label="Joined Kick" value={kickProfile.created_at ? formatDate(kickProfile.created_at) : null} />
                <ProviderStatRow label="Following since" value={kickProfile.following_since ? formatDate(kickProfile.following_since) : null} />
                <ProviderStatRow
                  label="Subscribed"
                  value={kickProfile.subscribed_for > 0 ? `${kickProfile.subscribed_for} month${kickProfile.subscribed_for === 1 ? '' : 's'}` : null}
                />
                <ProviderStatRow
                  label="Followers"
                  value={kickProfile.followers_count != null ? kickProfile.followers_count.toLocaleString() : null}
                />
                {kickProfile.is_banned && <ProviderStatRow label="Status" value="Banned in this channel" />}
              </div>
            )}
            {provider === 'youtube' && youtubeProfile && (
              <div className="glass-panel rounded-md px-3 py-2 space-y-1">
                <ProviderStatRow label="Subscribers" value={youtubeProfile.subscriber_count ?? null} />
              </div>
            )}

            {/* Badges worn in THIS room, straight off the message payload. */}
            {provider === 'kick' && !!kickProfile?.badges.length && (
              <div className="flex flex-wrap items-center gap-1.5">
                {kickProfile.badges.map((b) => (
                  <span
                    key={`${b.kind}-${b.text}`}
                    className="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] bg-white/[0.06] text-textSecondary"
                  >
                    {b.image_url && <img src={b.image_url} alt="" className="w-3.5 h-3.5" />}
                    {b.text}
                    {b.months ? ` · ${b.months}mo` : ''}
                  </span>
                ))}
              </div>
            )}

            {/* One action, and only one that actually works on that platform. */}
            {provider !== 'twitch' && (
              <button
                onClick={async () => {
                  const { open } = await import('@tauri-apps/plugin-shell');
                  const url =
                    provider === 'kick'
                      ? `https://kick.com/${kickProfile?.slug || login}`
                      : `https://www.youtube.com/channel/${youtubeProfile?.channel_id || userId}`;
                  await open(url).catch((e) => Logger.error('Failed to open channel:', e));
                }}
                className="glass-button w-full py-2 text-sm font-medium rounded-lg"
              >
                Open on {providerLabel(provider)}
              </button>
            )}

            {/* Relationship: a single compact stats panel + the sub/mod/vip pills.
                Each row only renders if we have the data, so the panel grows
                organically without empty cells. */}
            <div className="space-y-2">
              {(() => {
                // Current-channel context for "Gift a sub" target. Hidden when we're
                // viewing this user's own channel (you'd be gifting them a sub to
                // themselves, which is awkward) or when there's no active stream.
                const { channelName: viewedChannel } = (propChannelId && propChannelName)
                  ? { channelName: propChannelName }
                  : { channelName: useAppStore.getState().currentStream?.user_login };
                const canGiftSub = !!viewedChannel
                  && !!login
                  && viewedChannel.toLowerCase() !== login.toLowerCase()
                  && !ivrData?.is_subscribed;
                const handleGiftSub = async () => {
                  try {
                    const { open } = await import('@tauri-apps/plugin-shell');
                    // ?giftrecipient= is a best-guess query param. If Twitch ignores
                    // it the user lands on the subs page and types the name manually,
                    // which is still a one-click improvement over leaving the app.
                    await open(`https://www.twitch.tv/subs/${viewedChannel}?giftrecipient=${login}`);
                  } catch (err) {
                    Logger.error('Failed to open gift sub URL:', err);
                  }
                };
                // Every row defaults to on, so a user who never opens the
                // visibility settings sees the card exactly as before.
                const rows = {
                  joinDate: cardPrefs?.show_join_date !== false && !!twitchProfile,
                  followage: cardPrefs?.show_followage !== false
                    && !!ivrData
                    && (!!ivrData.following_since || ivrData.status_hidden || !profileLoading),
                  followsCount: cardPrefs?.show_follows_count !== false && (ivrData?.follows_count ?? 0) > 0,
                  chatterCount: cardPrefs?.show_chatter_count !== false && (ivrData?.chatter_count ?? 0) > 0,
                  pastSub: cardPrefs?.show_past_subscriber !== false
                    && !!ivrData && !ivrData.is_subscribed && (ivrData.sub_cumulative ?? 0) > 0,
                  lastLive: cardPrefs?.show_last_live !== false && !!ivrData?.last_broadcast_at,
                };
                const hasAnyStat = Object.values(rows).some(Boolean);
                return (
                  <>
                    {!ivrData && profileLoading && provider === 'twitch' && (
                      <div className="glass-panel rounded-md px-3 py-2.5 space-y-2" aria-hidden="true">
                        {[64, 52, 58].map((w) => (
                          <div key={w} className="flex items-center justify-between gap-3">
                            <span className="h-2.5 rounded bg-white/[0.06] animate-pulse" style={{ width: `${w - 24}%` }} />
                            <span className="h-2.5 w-1/4 rounded bg-white/[0.06] animate-pulse" />
                          </div>
                        ))}
                      </div>
                    )}
                    {hasAnyStat && (
                      <div className="glass-panel rounded-md px-3 py-2 space-y-1">
                        {rows.joinDate && (
                          <div className="flex items-baseline justify-between gap-3 text-[11px]">
                            <span className="text-textSecondary">Joined {providerLabel(provider)}</span>
                            <span className="text-textPrimary font-medium tabular-nums">
                              {formatDate(twitchProfile!.created_at)}
                              <RelativeSuffix iso={twitchProfile!.created_at} show={showRelative} />
                            </span>
                          </div>
                        )}
                        {rows.followage && (ivrData!.following_since || ivrData!.status_hidden) ? (
                          <div className="flex items-baseline justify-between gap-3 text-[11px]">
                            <span className="text-textSecondary">Following since</span>
                            {ivrData!.status_hidden ? (
                              <span className="text-textSecondary italic">Hidden</span>
                            ) : (
                              <span className="text-textPrimary font-medium tabular-nums">
                                {formatDate(ivrData!.following_since!)}
                                <RelativeSuffix iso={ivrData!.following_since!} show={showRelative} />
                              </span>
                            )}
                          </div>
                        ) : rows.followage && !profileLoading && (
                          <div className="flex items-baseline justify-between gap-3 text-[11px]">
                            <span className="text-textSecondary">Following</span>
                            <span className="text-textSecondary italic">Not following</span>
                          </div>
                        )}
                        {rows.followsCount && (
                          <div className="flex items-baseline justify-between gap-3 text-[11px]">
                            <span className="text-textSecondary">Follows</span>
                            <span className="text-textPrimary font-medium tabular-nums">{ivrData!.follows_count!.toLocaleString()} channels</span>
                          </div>
                        )}
                        {rows.chatterCount && (
                          <Tooltip content="People currently in this user's own chat" side="top">
                            <div className="flex items-baseline justify-between gap-3 text-[11px] cursor-default">
                              <span className="text-textSecondary">Chatters</span>
                              <span className="text-textPrimary font-medium tabular-nums">{ivrData!.chatter_count!.toLocaleString()}</span>
                            </div>
                          </Tooltip>
                        )}
                        {rows.pastSub && (
                          <div className="flex items-baseline justify-between gap-3 text-[11px]">
                            <span className="text-textSecondary">Past subscriber</span>
                            <span className="text-textPrimary font-medium tabular-nums">
                              {ivrData!.sub_cumulative === 1 ? '1mo total' : `${ivrData!.sub_cumulative}mo total`}
                            </span>
                          </div>
                        )}
                        {rows.lastLive && (
                          <Tooltip content={ivrData!.last_broadcast_title ?? formatDate(ivrData!.last_broadcast_at!)} side="top">
                            <div className="flex items-baseline justify-between gap-3 text-[11px] cursor-default">
                              <span className="text-textSecondary">Last live</span>
                              <span className="text-textPrimary font-medium tabular-nums">{formatRelative(ivrData!.last_broadcast_at!)}</span>
                            </div>
                          </Tooltip>
                        )}
                      </div>
                    )}

                    {cardPrefs?.show_notes !== false && (
                      <div className="glass-tile px-2.5 py-2">
                        <div className="mb-1 flex items-center justify-between">
                          <span className="text-[10px] font-semibold uppercase tracking-wider text-textSecondary">Private note</span>
                          <span className="text-[10px] text-textSecondary/70">
                            {noteSaved === 'saving' ? 'Saving…' : noteSaved === 'saved' ? 'Saved' : note ? 'Only you see this' : ''}
                          </span>
                        </div>
                        <textarea
                          value={note}
                          onChange={(e) => setNote(e.target.value.slice(0, 4000))}
                          onBlur={() => void saveNote()}
                          onClick={(e) => e.stopPropagation()}
                          rows={note.length > 60 ? 3 : 1}
                          placeholder="Add a note about this person…"
                          className="glass-input w-full resize-none px-2.5 py-1.5 text-xs leading-snug text-textPrimary placeholder-textSecondary/60 focus:outline-none"
                          spellCheck={false}
                        />
                      </div>
                    )}
                    {(ivrData?.is_subscribed || ivrData?.is_mod || ivrData?.is_vip) && (
                      <div className="flex flex-wrap gap-1.5">
                        {ivrData?.is_subscribed && (
                          <span className="inline-flex items-center gap-1.5 px-2 py-1 rounded-md bg-accent/10 border border-accent/20 text-[11px] text-accent">
                            <span className="font-semibold">
                              {ivrData.sub_tier === 'Prime' ? 'Prime' : ivrData.sub_tier ? `Tier ${ivrData.sub_tier}` : ''}
                              {ivrData.sub_tier ? ' ' : ''}Subscriber
                            </span>
                            <span className="text-accent/70">{formatSubTenure(ivrData.sub_streak, ivrData.sub_cumulative)}</span>
                            {ivrData.sub_type === 'gift' && ivrData.sub_gifter_display_name && (
                              <span className="text-accent/70">gift from {ivrData.sub_gifter_display_name}</span>
                            )}
                            {ivrData.is_founder && <span className="px-1 py-px rounded bg-accent/20 text-[9px] font-bold tracking-wider">FOUNDER</span>}
                          </span>
                        )}
                        {ivrData?.is_mod && (
                          <span className="inline-flex items-center gap-1.5 px-2 py-1 rounded-md bg-success/10 border border-success/20 text-[11px] text-success">
                            <span className="font-semibold">Moderator</span>
                            {ivrData.mod_since && (
                              <span className="text-success/70">
                                since {formatIVRDate(ivrData.mod_since)}
                                {showRelative && ` (${formatRelative(ivrData.mod_since)})`}
                              </span>
                            )}
                          </span>
                        )}
                        {ivrData?.is_vip && (
                          <span className="inline-flex items-center gap-1.5 px-2 py-1 rounded-md bg-highlight-pink/10 border border-highlight-pink/20 text-[11px] text-highlight-pink">
                            <span className="font-semibold">VIP</span>
                            {ivrData.vip_since && (
                              <span className="text-pink-300/70">
                                since {formatIVRDate(ivrData.vip_since)}
                                {showRelative && ` (${formatRelative(ivrData.vip_since)})`}
                              </span>
                            )}
                          </span>
                        )}
                      </div>
                    )}

                    {/* Gift-sub action. Text link, not a chip — keeps the
                        visual weight light and matches the "View Full Changelog"
                        / "Open Settings →" pattern. Hover state adds underline +
                        a small arrow nudge so it reads as clickable without
                        becoming a border-pill. */}
                    {canGiftSub && (
                      <button
                        type="button"
                        onClick={handleGiftSub}
                        className="group inline-flex items-center gap-1.5 text-[11px] text-accent hover:text-accent/90 hover:underline underline-offset-[3px] decoration-accent/60 transition-colors"
                      >
                        <Gift size={12} />
                        <span>Gift {displayName} a sub to {viewedChannel}</span>
                        <span aria-hidden="true" className="transition-transform group-hover:translate-x-0.5">→</span>
                      </button>
                    )}
                  </>
                );
              })()}
            </div>

            {!isMemberCard && badgesPanel}

            {/* Personal nickname + chat color (only visible to this user).
                Doesn't change @mention behavior — Twitch IRC still resolves
                real logins, and color is purely a display-layer override. */}
            {/* The override key must match what the chat renderer LOOKS UP, which
                is the namespaced cosmetics key for a provider chatter and the bare
                id for Twitch. Writing a bare Kick/YouTube id here would save an
                override that could never be found again. */}
            <NicknameEditor
              userId={provider && provider !== 'twitch' ? `${provider}:${userId}` : userId}
              username={username}
              displayName={displayName}
              twitchColor={color}
            />

            {!isMemberCard && actionsBlock}

            {/* Chat filters: hide this user's messages locally, in this channel
                or everywhere. Available to every viewer (not a mod action, and
                nothing is sent to the platform); hidden for your own card since
                own messages are exempt at ingest anyway. */}
            {!targetIsSelf && username && (
              <ChatFilterActions
                username={username}
                displayName={displayName}
                provider={provider ?? 'twitch'}
                channel={getChannelContext().channelName || ''}
              />
            )}

            {/* YouTube mod zone. Addressed by CHANNEL IDENTIFIER (what channel_meta
                is keyed by, i.e. propChannelName / user_login) and the chatter's
                UC id — not the broadcaster id, which is a different value here.
                YouTube's timeout is a single fixed length, so there is one Timeout
                button rather than a duration grid; "Hide" is its permanent ban. */}
            {isModerator && provider === 'youtube' && propChannelName && !targetIsSelf && (
              <div className="pt-3 border-t border-error/20">
                <div className="flex items-center gap-1.5 mb-2.5">
                  <svg className="w-3.5 h-3.5 text-error" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z"/><path d="M12 9v4"/><path d="M12 17h.01"/></svg>
                  <span className="text-[10px] uppercase font-bold text-error tracking-wider">Moderator Actions</span>
                </div>
                <div className="grid grid-cols-3 gap-2">
                  {([
                    { label: 'Timeout', kind: 'timeout', title: 'Time out (YouTube sets the length)' },
                    { label: 'Hide', kind: 'ban', title: 'Hide this user from chat permanently' },
                    { label: 'Unhide', kind: 'unban', title: 'Let this user back into chat' },
                  ] as const).map((action) => (
                    <Tooltip key={action.label} content={action.title} side="top">
                      <button
                        onClick={async () => {
                          try {
                            const { invoke } = await import('@tauri-apps/api/core');
                            const args = { channel: propChannelName, targetChannelId: userId };
                            if (action.kind === 'unban') {
                              await invoke('youtube_unban_user', args);
                            } else {
                              await invoke('youtube_ban_user', {
                                ...args,
                                durationSeconds: action.kind === 'timeout' ? 300 : null,
                              });
                            }
                            useAppStore
                              .getState()
                              .addToast(
                                action.kind === 'unban'
                                  ? `${displayName || login} can chat again`
                                  : action.kind === 'timeout'
                                    ? `Timed out ${displayName || login}`
                                    : `Hid ${displayName || login} from chat`,
                                'success',
                              );
                            onClose();
                          } catch (err) {
                            Logger.error('[UserProfileCard] YouTube mod action failed:', err);
                            useAppStore
                              .getState()
                              .addToast(`Couldn't ${action.label.toLowerCase()} ${displayName || login}`, 'error');
                          }
                        }}
                        className={`w-full py-1.5 glass-button text-xs font-semibold text-white/70 hover:text-white rounded flex items-center justify-center transition-colors border ${
                          action.kind === 'ban'
                            ? 'hover:bg-error/20 hover:border-error/30'
                            : 'hover:bg-warning/20 hover:border-warning/30'
                        }`}
                      >
                        {action.label}
                      </button>
                    </Tooltip>
                  ))}
                </div>
              </div>
            )}

            {/* Kick mod zone. Kick has no VIP/mod-grant API for us and no
                cross-mod protection rule to explain, so it is the four actions
                that DO exist rather than a greyed-out copy of Twitch's grid.
                Durations are whole minutes because that is Kick's unit. */}
            {isModerator && broadcasterId && provider === 'kick' && !targetIsSelf && (
              <div className="pt-3 border-t border-error/20">
                <div className="flex items-center gap-1.5 mb-2.5">
                  <svg className="w-3.5 h-3.5 text-error" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z"/><path d="M12 9v4"/><path d="M12 17h.01"/></svg>
                  <span className="text-[10px] uppercase font-bold text-error tracking-wider">Moderator Actions</span>
                </div>
                <div className="grid grid-cols-4 gap-2">
                  {([
                    { label: '10m', minutes: 10, title: 'Time out for 10 minutes', danger: false },
                    { label: '24h', minutes: 1440, title: 'Time out for 24 hours', danger: false },
                    { label: 'Ban', minutes: null, title: 'Ban permanently', danger: true },
                    { label: 'Unban', minutes: undefined, title: 'Remove a ban or timeout', danger: false },
                  ] as const).map((action) => (
                    <Tooltip key={action.label} content={action.title} side="top">
                      <button
                        onClick={async () => {
                          try {
                            const { invoke } = await import('@tauri-apps/api/core');
                            const ids = {
                              broadcasterUserId: Number(broadcasterId),
                              targetUserId: Number(userId),
                            };
                            if (!Number.isFinite(ids.broadcasterUserId) || !Number.isFinite(ids.targetUserId)) {
                              useAppStore.getState().addToast('Kick did not give us an id for that user', 'error');
                              return;
                            }
                            if (action.minutes === undefined) {
                              await invoke('kick_unban_user', ids);
                              useAppStore.getState().addToast(`Lifted the ban on ${login}`, 'success');
                            } else {
                              await invoke('kick_ban_user', {
                                ...ids,
                                durationMinutes: action.minutes,
                                reason: null,
                              });
                              useAppStore
                                .getState()
                                .addToast(
                                  action.minutes === null
                                    ? `Banned ${login}`
                                    : `Timed out ${login} for ${action.label}`,
                                  'success',
                                );
                            }
                            onClose();
                          } catch (err) {
                            Logger.error('[UserProfileCard] Kick mod action failed:', err);
                            useAppStore.getState().addToast(`Couldn't ${action.label.toLowerCase()} ${login}`, 'error');
                          }
                        }}
                        className={`w-full py-1.5 glass-button text-xs font-semibold text-white/70 hover:text-white rounded flex items-center justify-center transition-colors border ${
                          action.danger
                            ? 'hover:bg-error/20 hover:border-error/30'
                            : 'hover:bg-warning/20 hover:border-warning/30'
                        }`}
                      >
                        {action.label}
                      </button>
                    </Tooltip>
                  ))}
                </div>
              </div>
            )}

            {/* Mod zone. Kept inside body container; red top border is the only divider we keep, as a semantic danger-zone signal */}
            {isModerator && broadcasterId && provider === 'twitch' && (
              <div className="pt-3 border-t border-error/20">
                <div className="flex items-center gap-1.5 mb-2.5">
                  <svg className="w-3.5 h-3.5 text-error" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z"/><path d="M12 9v4"/><path d="M12 17h.01"/></svg>
                  <span className="text-[10px] uppercase font-bold text-error tracking-wider">Moderator Actions</span>
                </div>
                {blockedTarget && (
                  <p className="mb-2.5 text-[11px] text-textSecondary">
                    {targetIsBroadcaster
                      ? "This is the channel's owner, so there's nothing to action here."
                      : 'Twitch does not let one moderator action another. Only the broadcaster can.'}
                  </p>
                )}
                {!blockedTarget && (
                <div className="grid grid-cols-4 gap-2">
                  <Tooltip content="Purge recent messages" side="top">
                    <button
                      onClick={async () => {
                        if (onPreFillCommand) { onPreFillCommand(`/timeout ${login} 1 Purge`); onClose(); return; }
                        try {
                          const { invoke } = await import('@tauri-apps/api/core');
                          await invoke('ban_user', { broadcasterId, targetUserId: userId, duration: 1, reason: 'Purge' });
                          useAppStore.getState().addToast(`Purged messages for ${login}`, 'success');
                        } catch (err) {
                          Logger.error('[UserProfileCard] Failed to purge:', err);
                          useAppStore.getState().addToast('Failed to purge user', 'error');
                        }
                      }}
                      className="py-1.5 glass-button text-xs font-semibold text-white/70 hover:text-white hover:bg-warning/20 border hover:border-warning/30 rounded flex items-center justify-center transition-colors"
                    >
                      Purge
                    </button>
                  </Tooltip>
                  <Tooltip content="Timeout for 10 minutes" side="top">
                    <button
                      onClick={async () => {
                        if (onPreFillCommand) { onPreFillCommand(`/timeout ${login} 600 ${defaultBanReason}`); onClose(); return; }
                        try {
                          const { invoke } = await import('@tauri-apps/api/core');
                          await invoke('ban_user', { broadcasterId, targetUserId: userId, duration: 600, reason: null });
                          useAppStore.getState().addToast(`Timed out ${login} for 10m`, 'success');
                        } catch (err) {
                          Logger.error('[UserProfileCard] Failed to timeout:', err);
                          useAppStore.getState().addToast('Failed to timeout user', 'error');
                        }
                      }}
                      className="py-1.5 glass-button text-xs font-semibold text-white/70 hover:text-white hover:bg-warning/20 border hover:border-yellow-500/30 rounded flex items-center justify-center transition-colors"
                    >
                      10m
                    </button>
                  </Tooltip>
                  <Tooltip content="Timeout for 24 hours" side="top">
                    <button
                      onClick={async () => {
                        if (onPreFillCommand) { onPreFillCommand(`/timeout ${login} 86400 ${defaultBanReason}`); onClose(); return; }
                        try {
                          const { invoke } = await import('@tauri-apps/api/core');
                          await invoke('ban_user', { broadcasterId, targetUserId: userId, duration: 86400, reason: null });
                          useAppStore.getState().addToast(`Timed out ${login} for 24h`, 'success');
                        } catch (err) {
                          Logger.error('[UserProfileCard] Failed to timeout:', err);
                          useAppStore.getState().addToast('Failed to timeout user', 'error');
                        }
                      }}
                      className="py-1.5 glass-button text-xs font-semibold text-white/70 hover:text-white hover:bg-warning/25 border hover:border-warning/35 rounded flex items-center justify-center transition-colors"
                    >
                      24h
                    </button>
                  </Tooltip>
                  <Tooltip content="Permanently Ban User" side="top">
                    <button
                      onClick={async () => {
                        if (onPreFillCommand) { onPreFillCommand(`/ban ${login} ${defaultBanReason}`); onClose(); return; }
                        if (window.confirm(`Are you sure you want to permanently ban ${login}?`)) {
                          try {
                            const { invoke } = await import('@tauri-apps/api/core');
                            await invoke('ban_user', { broadcasterId, targetUserId: userId, duration: null, reason: null });
                            useAppStore.getState().addToast(`Banned ${login}`, 'success');
                            onClose();
                          } catch (err) {
                            Logger.error('[UserProfileCard] Failed to ban:', err);
                            useAppStore.getState().addToast('Failed to ban user', 'error');
                          }
                        }
                      }}
                      className="py-1.5 text-xs font-semibold text-red-200 bg-red-900/50 hover:bg-red-600 border border-red-500/30 rounded flex items-center justify-center transition-colors shadow-lg"
                    >
                      Ban
                    </button>
                  </Tooltip>
                </div>
                )}
                {/* Role controls. Twitch restricts these to the channel owner,
                    so they only appear for the broadcaster. The commands and
                    scopes already existed; this is the button surface. */}
                {viewerIsBroadcaster && !targetIsBroadcaster && (
                  <div className="mt-2 grid grid-cols-2 gap-2">
                    <RoleActionButton
                      label={targetIsModerator ? 'Unmod' : 'Mod'}
                      tooltip={targetIsModerator ? `Remove ${login} as a moderator` : `Make ${login} a moderator`}
                      command={targetIsModerator ? 'remove_channel_moderator' : 'add_channel_moderator'}
                      broadcasterId={broadcasterId}
                      userId={userId}
                      successText={targetIsModerator ? `Removed ${login} as moderator` : `${login} is now a moderator`}
                    />
                    <RoleActionButton
                      label={ivrData?.is_vip ? 'Remove VIP' : 'VIP'}
                      tooltip={ivrData?.is_vip ? `Remove VIP from ${login}` : `Give ${login} VIP`}
                      command={ivrData?.is_vip ? 'remove_channel_vip' : 'add_channel_vip'}
                      broadcasterId={broadcasterId}
                      userId={userId}
                      successText={ivrData?.is_vip ? `Removed VIP from ${login}` : `${login} is now a VIP`}
                    />
                  </div>
                )}
                <div className="mt-2 text-right">
                  <button
                    onClick={async () => {
                      if (window.confirm(`Unban ${login}?`)) {
                        try {
                          const { invoke } = await import('@tauri-apps/api/core');
                          await invoke('unban_user', { broadcasterId, targetUserId: userId });
                          useAppStore.getState().addToast(`Unbanned ${login}`, 'success');
                        } catch (err) {
                          Logger.error('[UserProfileCard] Failed to unban:', err);
                          useAppStore.getState().addToast('Failed to unban user', 'error');
                        }
                      }
                    }}
                    className="text-[10px] font-medium text-textSecondary hover:text-white hover:underline cursor-pointer"
                  >
                    Unban User
                  </button>
                </div>
              </div>
            )}

                  </motion.div>
                )}
              </AnimatePresence>
            </div>
          </motion.div>
        </div>
      </div>
    </>
  );
};

export default UserProfileCard;
