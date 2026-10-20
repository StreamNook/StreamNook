import { Fragment, useState, useEffect, useLayoutEffect, useRef, useCallback, useMemo } from 'react';
import { createPortal } from 'react-dom';
import { motion, AnimatePresence } from 'framer-motion';
import { Bell, BellOff, Radio, MessageCircle, User, Download, Award, Check, CheckCheck, Info, CheckCircle2, AlertTriangle, XCircle, AtSign, Reply, Highlighter } from 'lucide-react';
import { X, SpeakerSlash, Trash, PuzzlePiece, Package, Gift, Sparkle } from 'phosphor-react';
import { listen, emit } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore, type SettingsTab } from '../stores/AppStore';
import { useTitleBarLifted } from '../hooks/useTitleBarAutoHide';
import { exitPlayerFullscreenForBarAction } from '../utils/windowFullscreen';
import { requestChangelog } from '../utils/changelogEvents';
import { requestWallet } from '../utils/walletRequest';
import { openMentionInMain } from '../stores/mentionJumpStore';
import { MENTION_INBOX_CHANGED_EVENT, MENTION_NOTIFY_EVENT, toggleMentionChatMuted } from '../stores/mentionInboxStore';
import { isProviderId } from '../types/providers';
import { Logger } from '../utils/logger';
import { formatBadgeDateInfo } from '../utils/badgeWindow';
import { windowStatusAt, type WindowRun } from '../services/badgeStanding';
import { playNotificationSound as playGatedNotificationSound } from '../utils/notificationSound';
import { liveActivityText } from '../utils/liveActivity';
import { IS_MOBILE } from '../utils/platform';
import {
    MIRRORED_NOTIFICATION_ACTIVATE_EVENT,
    type MirroredNotice,
    type MirroredNoticeKind,
    NOTICE_ACTION_PENDING_EVENT,
    type NoticeAction,
    type NoticeReply,
} from '../utils/notificationMirror';
import { Tooltip } from './ui/Tooltip';
import { ProviderIcon } from './overlay/ProviderIcon';
import { SevenTvAvatar } from './SevenTvAvatar';
import { islandSurfaceGeometry } from '../utils/islandGeometry';
import type { ProviderId } from '../types/providers';
import type {
    DynamicIslandNotification,
    LiveNotificationData,
    WhisperNotificationData,
    UpdateNotificationData,
    DropsNotificationData,
    ChannelPointsNotificationData,
    BadgeNotificationData,
    SystemNotificationData,
    GiftSubNotificationData,
    TwitchRewardNotificationData,
    MembershipGiftNotificationData,
    MentionEntry,
    MentionKind,
    MentionPage,
    WhisperUpdate,
} from '../types';

const MAX_NOTIFICATIONS = 20;
const CACHE_KEY = 'streamnook_notifications';
const CACHE_EXPIRY_DAYS = 7;
// How long a "Test" button dummy notification lingers in the notification
// center before it auto-removes, so previews don't pile up with real ones.
// Kept >= the live preview hold (8s) so the entry outlasts its own preview.
const TEST_NOTIFICATION_TTL_MS = 8000;
// Click actions kept for notifications shown in a MultiChat window.
const MAX_MIRRORED_ACTIONS = 12;
// Mentions come from Rust's inbox a page at a time; scrolling to the end of
// the history asks for the next older page.
const MENTION_PAGE = 20;
/** `get_mention_inbox` caps a page here. */
const MENTION_PAGE_MAX = 200;
// The open panel with nothing in it: the header (42) and the empty state
// (its ring, line and padding).
const EMPTY_PANEL_HEIGHT = 42 + 190;

/** A mention or reply from Rust's inbox, drawn in the history beside the
 *  island's own notifications. Never cached here: Rust owns the entry and its
 *  read state, so these are re-read from it, not saved with the rest. */
interface MentionItem {
    id: string;
    type: 'mention';
    timestamp: number;
    read: boolean;
    data: MentionEntry;
}
type IslandItem = DynamicIslandNotification | MentionItem;

const toMentionItem = (entry: MentionEntry): MentionItem => ({
    id: `mention:${entry.id}`,
    type: 'mention',
    timestamp: entry.at_ms,
    read: entry.read,
    data: entry,
});

const MENTION_KIND_WORDS: Record<MentionKind, string> = {
    mention: 'mentioned you',
    reply: 'replied to you',
    word: 'matched a highlight',
};

// The kind is the mark: an @ for a mention, the reply arrow for a reply, the
// highlighter for one of your highlight words.
const MentionGlyph = ({ kind, size }: { kind: MentionKind; size: number }) => {
    const Icon = kind === 'reply' ? Reply : kind === 'word' ? Highlighter : AtSign;
    // The reply arrow's ink is short and wide; a step up matches the @'s weight.
    return <Icon size={kind === 'reply' ? Math.round(size * 1.2) : size} className="text-accent flex-shrink-0" />;
};

// A mention's row in the history: who, in which chat, and what they said.
// The row around it (hover, read dimming, time and actions) is the island's.
const MentionRowBody = ({ entry, showPlatform }: { entry: MentionEntry; showPlatform: boolean }) => {
    const platform = showPlatform && isProviderId(entry.platform) ? entry.platform : null;
    return (
        <>
            <div className="relative flex-shrink-0">
                <div className="sn-notif-glyph">
                    <MentionGlyph kind={entry.kind} size={20} />
                </div>
            </div>
            <div className="flex-1 min-w-0">
                <div className="flex items-center gap-1.5 min-w-0">
                    {/* Rust stores the color readable on the CHAT's background;
                        the panel is always dark, so its lightness is lifted to
                        read there (a browser without relative colors keeps
                        plain white). */}
                    <span
                        className="text-sm font-semibold truncate min-w-0 max-w-[60%] text-white"
                        style={{ color: entry.sender_color ? `oklch(from ${entry.sender_color} max(l, 0.75) c h)` : undefined }}
                    >
                        {entry.sender_name}
                    </span>
                    <span className="sr-only">{MENTION_KIND_WORDS[entry.kind]}</span>
                    {/* Smaller type than the name, nudged onto its baseline. */}
                    <span className="flex-shrink-0 translate-y-px text-[12.5px] text-white/40">in</span>
                    <span className="flex min-w-0 items-center gap-1 text-[12.5px] text-white/70">
                        {platform && <ProviderIcon provider={platform} size="12px" />}
                        <span className="truncate translate-y-px">{entry.channel_login}</span>
                    </span>
                </div>
                {entry.removed ? (
                    <p className="text-white/40 text-sm italic mt-0.5">Message deleted by a moderator</p>
                ) : (
                    <p className="text-white/50 text-sm line-clamp-2 break-words mt-0.5">{entry.text}</p>
                )}
            </div>
        </>
    );
};


interface TwitchRewardFromBackend {
    id: string;
    kind: 'badge' | 'drop';
    body_plain: string;
    body_spans: { text: string; bold: boolean }[];
    created_at: string;
    thumbnail_url: string;
    action_url: string | null;
}

interface GiftSubFromBackend {
    id: string;
    body_plain: string;
    body_spans: { text: string; bold: boolean }[];
    created_at: string;
    thumbnail_url: string;
    channel_login: string | null;
    action_url: string | null;
}

interface LiveNotificationFromBackend {
    streamer_name: string;
    streamer_login: string;
    streamer_avatar?: string;
    game_name?: string;
    game_image?: string;
    stream_title?: string;
    stream_url: string;
    is_test?: boolean;
    /** Which watcher raised this. Absent = the follow poller. 'favorite' = the
     *  favorites sweep, which has its own setting: a channel you favorited but
     *  don't follow must not be silenced by the follows toggle, or the reverse. */
    source?: string;
    /** From Rust's go-live gate (services/live_announce.rs), which has already
     *  dropped repeats: the entry this is, every platform the person is live
     *  on, and the open entry it joins when it is another platform's go-live. */
    notification_id: string;
    providers: ProviderId[];
    merge_into: string | null;
    /** The streamer's 7TV cosmetics id, keyed by Rust for their platform. */
    seventv_id?: string | null;
}

interface BundleUpdateStatus {
    update_available: boolean;
    current_version: string;
    latest_version: string;
    download_url: string | null;
    bundle_name: string | null;
    download_size: string | null;
    releases_behind?: number | null;
}

interface DropClaimedEvent {
    drop_name: string;
    game_name: string;
    benefit_name?: string;
    benefit_image_url?: string;
}

/** A burst of channel-points earns, gathered by Rust
 *  (services/channel_points_summary.rs). Channels are busiest first, each with
 *  its face, its own name for its points and its points icon when Rust knows
 *  them (services/points_notify.rs). Rust also raises the system notification
 *  for it, so this only presents the app's own toast and list row. */
interface ChannelPointsSummary {
    total_points: number;
    channels: Array<{
        name: string;
        points: number;
        avatar_url?: string;
        points_name?: string;
        icon_url?: string;
    }>;
    reasons: Array<{ name: string; points: number }>;
    first_reason: string;
    last_balance: number | null;
}

/** Settings tabs a notification may open (a notification link can come from
 *  anywhere, so only these). */
const SETTINGS_TABS_FROM_NOTICES: SettingsTab[] = ['Notifications'];

// Cache helpers
const loadCachedNotifications = (): DynamicIslandNotification[] => {
    try {
        const cached = localStorage.getItem(CACHE_KEY);
        if (!cached) return [];

        const { notifications } = JSON.parse(cached);
        const expiryTime = CACHE_EXPIRY_DAYS * 24 * 60 * 60 * 1000;

        // Filter out notifications older than expiry time
        const validNotifications = notifications.filter(
            (n: DynamicIslandNotification) => Date.now() - n.timestamp < expiryTime
        );

        return validNotifications.slice(0, MAX_NOTIFICATIONS);
    } catch {
        return [];
    }
};

const saveCachedNotifications = (notifications: DynamicIslandNotification[]) => {
    try {
        localStorage.setItem(CACHE_KEY, JSON.stringify({
            notifications: notifications.slice(0, MAX_NOTIFICATIONS),
            timestamp: Date.now(),
        }));
    } catch (error) {
        Logger.warn('Failed to cache notifications:', error);
    }
};

// The one-line text shown in the collapsed preview pill. Centralized so the
// width-measuring code sizes the pill to exactly what gets rendered.
const getPreviewText = (n: IslandItem): string => {
    switch (n.type) {
        case 'mention': {
            // Worded like the system notification Rust raises for the same entry.
            const { sender_name: name, channel_login: channel, kind } = n.data;
            if (kind === 'reply') return `${name} replied to you in ${channel}`;
            if (kind === 'word') return `${name} in ${channel}`;
            return `${name} mentioned you in ${channel}`;
        }
        case 'live': {
            const d = n.data as LiveNotificationData;
            return d.game_name ? `${d.streamer_name} is live • ${d.game_name}` : `${d.streamer_name} is live`;
        }
        case 'whisper':
            return `Whisper from ${(n.data as WhisperNotificationData).from_user_name}`;
        case 'update':
            return 'Update available';
        case 'drops':
            return 'Drop claimed';
        case 'channel_points': {
            const d = n.data as ChannelPointsNotificationData;
            const base = `+${d.points_earned.toLocaleString()} ${d.points_name || 'Channel Points'}`;
            // channel_name can hold a reason summary like "+20 (watching)" rather
            // than a real channel; only append it when it's a real channel.
            const isReasonSummary = !!d.channel_name && d.channel_name.includes('(');
            return d.channel_name && !isReasonSummary ? `${base} • ${d.channel_name}` : base;
        }
        case 'badge':
            return (n.data as BadgeNotificationData).badge_name;
        case 'gift_sub':
            return (n.data as GiftSubNotificationData).body_plain;
        case 'twitch_reward':
            return (n.data as TwitchRewardNotificationData).body_plain;
        case 'membership_gift':
            return 'You were given a StreamNook membership';
        case 'system':
            return (n.data as SystemNotificationData).message;
        default:
            return '';
    }
};

// Twitch's channel points mark, for a burst with no single face to show.
const PointsGlyph = ({ size }: { size: number }) => (
    <svg width={size} height={size} viewBox="0 0 24 24" className="text-orange-400 flex-shrink-0" fill="currentColor">
        <path d="M12 5v2a5 5 0 0 1 5 5h2a7 7 0 0 0-7-7Z"></path>
        <path fillRule="evenodd" d="M1 12C1 5.925 5.925 1 12 1s11 4.925 11 11-4.925 11-11 11S1 18.075 1 12Zm11 9a9 9 0 1 1 0-18 9 9 0 0 1 0 18Z" clipRule="evenodd"></path>
    </svg>
);

// A channel's face with its points icon attached at the corner, the same
// seal the level uses (a dark outline, no chip). Without a custom icon the
// seal is Twitch's own mark.
const PointsFace = ({ avatarUrl, iconUrl }: { avatarUrl: string; iconUrl?: string }) => (
    <div className="relative flex-shrink-0 w-[34px] h-[34px]">
        <img src={avatarUrl} alt="" className="w-[34px] h-[34px] rounded-full object-cover" />
        <span className="sn-notif-seal">
            {iconUrl
                ? <img src={iconUrl} alt="" className="w-4 h-4 object-contain" />
                : <PointsGlyph size={14} />}
        </span>
    </div>
);

// The leading mark for the collapsed preview AND for the stacked faces on the
// trigger, which is why it has to agree with the rows in the list below: a
// profile picture wherever there is one (a streamer, a whisper sender, you), the
// source glyph for anything a plugin raised, and the type or level glyph
// otherwise.
const renderPreviewIcon = (n: IslandItem): React.ReactNode => {
    switch (n.type) {
        case 'mention':
            // Rust keeps no picture of the sender, so the kind's mark.
            return <MentionGlyph kind={n.data.kind} size={11} />;
        case 'live': {
            const d = n.data as LiveNotificationData;
            return d.streamer_avatar
                ? <SevenTvAvatar cosmeticsId={d.seventv_id} px={16} src={d.streamer_avatar} alt="" className="w-4 h-4 rounded-full object-cover flex-shrink-0" />
                : <div className="w-1.5 h-1.5 bg-red-500 rounded-full flex-shrink-0" />;
        }
        case 'whisper': {
            const d = n.data as WhisperNotificationData;
            // Whispers are Twitch's, so the sender id is a bare Twitch id.
            return d.profile_image_url
                ? <SevenTvAvatar cosmeticsId={d.from_user_id} px={16} src={d.profile_image_url} alt="" className="w-4 h-4 rounded-full object-cover flex-shrink-0" />
                : <MessageCircle size={11} className="text-purple-400 flex-shrink-0" />;
        }
        case 'update':
            return <Download size={11} className="text-yellow-400 flex-shrink-0" />;
        case 'drops':
            return <Package size={11} className="text-green-400 flex-shrink-0" />;
        case 'channel_points': {
            const d = n.data as ChannelPointsNotificationData;
            return d.avatar_url
                ? <img src={d.avatar_url} alt="" className="w-4 h-4 rounded-full object-cover flex-shrink-0" />
                : <PointsGlyph size={11} />;
        }
        case 'badge':
            return <Award size={11} className="text-cyan-400 flex-shrink-0" />;
        case 'gift_sub': {
            // The thumbnail is the gifter's avatar, so it wins over the glyph
            // for the same reason every other row prefers a face.
            const d = n.data as GiftSubNotificationData;
            return d.thumbnail_url
                ? <img src={d.thumbnail_url} alt="" className="w-4 h-4 rounded-full object-cover flex-shrink-0" />
                : <Gift size={11} className="text-pink-400 flex-shrink-0" />;
        }
        case 'twitch_reward': {
            const d = n.data as TwitchRewardNotificationData;
            return d.thumbnail_url
                ? <img src={d.thumbnail_url} alt="" className="w-4 h-4 rounded object-contain flex-shrink-0" />
                : <Award size={11} className="text-cyan-400 flex-shrink-0" />;
        }
        case 'membership_gift':
            return <Sparkle size={11} className="text-amber-400 flex-shrink-0" weight="fill" />;
        case 'system': {
            const d = n.data as SystemNotificationData;
            // Same order the row uses, for the same reason: a face beats a
            // glyph, and a source beats a status. This drives BOTH the stacked
            // faces on the trigger and the pop-up preview, so the two can never
            // disagree with the list they came from.
            if (d.avatar_url) {
                return <SevenTvAvatar cosmeticsId={d.avatar_seventv_id} px={16} src={d.avatar_url} alt="" className="w-4 h-4 rounded-full object-cover flex-shrink-0" />;
            }
            if (d.source === 'plugin') return <PuzzlePiece size={11} className="text-accent flex-shrink-0" />;
            if (d.level === 'success') return <CheckCircle2 size={11} className="text-green-400 flex-shrink-0" />;
            if (d.level === 'error') return <XCircle size={11} className="text-red-400 flex-shrink-0" />;
            if (d.level === 'warning') return <AlertTriangle size={11} className="text-yellow-400 flex-shrink-0" />;
            return <Info size={11} className="text-accent flex-shrink-0" />;
        }
        default:
            return null;
    }
};

const DynamicIsland = () => {
    const [isExpanded, setIsExpanded] = useState(false);
    const [notifications, setNotifications] = useState<DynamicIslandNotification[]>(() => loadCachedNotifications());
    const [latestNotification, setLatestNotification] = useState<IslandItem | null>(null);
    const [showPreview, setShowPreview] = useState(false);
    const [windowSize, setWindowSize] = useState({ width: window.innerWidth, height: window.innerHeight });
    // Measured width of the current preview's content, so the pill grows to fit
    // the whole message instead of truncating (see the useLayoutEffect below).
    const [previewWidth, setPreviewWidth] = useState(200);
    const previewTextRef = useRef<HTMLSpanElement>(null);
    const islandRef = useRef<HTMLDivElement>(null);
    // The title bar's slot for the resting trigger, and the trigger itself.
    // Portaling into the title bar rather than rendering the trigger there
    // keeps every notification in this component: nothing has to be lifted into
    // a store just so two places can draw the same list.
    const [slot, setSlot] = useState<HTMLElement | null>(null);
    const triggerRef = useRef<HTMLButtonElement>(null);
    // Where the surface grows FROM. Null until measured, and on mobile (no
    // title bar, so no slot) it stays null and the old centred behaviour holds.
    const [anchor, setAnchor] = useState<
        {
            left: number; top: number; width: number; height: number; cover: number; radius: number;
            // The stretch of the bar the surface may use: from the left icon
            // cluster's right edge to the right cluster's left edge, less the
            // bar's gap on each side.
            roomLeft: number; roomRight: number;
        } | null
    >(null);
    const previewTimeoutRef = useRef<NodeJS.Timeout | null>(null);

    // Track current time for "X ago" formatting - updated every 30 seconds
    // This avoids calling Date.now() during render (impure function)
    const [now, setNow] = useState(() => Date.now());
    useEffect(() => {
        const interval = setInterval(() => setNow(Date.now()), 30000);
        return () => clearInterval(interval);
    }, []);

    // Actions are stable, so read them without subscribing; only the two state
    // fields drive re-renders now (this was a whole-store subscription).
    const { startStream, openWhisperWithUser, openSettings, addToast, setShowDropsOverlay, setShowBadgesOverlay } = useAppStore.getState();
    const settings = useAppStore((s) => s.settings);
    const isSettingsOpen = useAppStore((s) => s.isSettingsOpen);
    const isPlayerFullscreen = useAppStore((s) => s.isPlayerFullscreen);
    // The bar lifts over the player in its tucking modes and in the player's
    // full screen; the surface grows out of the bar's door, so it lifts with
    // it or the door would draw over it.
    const barLifted = useTitleBarLifted();

    // Two homes, and which one is right changes as you navigate.
    //
    // With the Home strip on screen the trigger belongs INSIDE it, at the left
    // end behind a hairline, and the notification expands to fill the strip.
    // With no strip (watching a stream, a category drill-down, a phone) it is
    // its own glazed pill floating in the bar, and expands from its own centre.
    //
    // The inline slot is Home's, so it appears and disappears as Home mounts and
    // unmounts. A one-shot lookup would bind to whichever happened to exist at
    // start-up and then be wrong for the rest of the session, so watch the nav
    // slot (which is the title bar's own and always there) for the strip
    // arriving or leaving.
    useEffect(() => {
        const outer = document.getElementById('sn-island-slot');
        const resolve = () =>
            setSlot(document.getElementById('sn-island-slot-inline') ?? outer);
        resolve();
        const host = document.getElementById('sn-nav-slot');
        if (!host) return;
        const mo = new MutationObserver(resolve);
        mo.observe(host, { childList: true, subtree: true });
        return () => mo.disconnect();
    }, []);

    // The slot moves whenever the centred group around it changes width -- the
    // nav pill gaining a Results tab, say -- and that is a POSITION change with
    // no size change, which a ResizeObserver on the slot alone would miss. So
    // watch the parent too.
    useLayoutEffect(() => {
        if (!slot) return;
        const leftCluster = document.querySelector<HTMLElement>('[data-titlebar-left]');
        const rightCluster = document.querySelector<HTMLElement>('[data-titlebar-right]');
        const measure = () => {
            const r = slot.getBoundingClientRect();
            const bar = leftCluster?.parentElement;
            const gap = bar ? parseFloat(getComputedStyle(bar).columnGap) || 0 : 0;
            const roomLeft = leftCluster ? leftCluster.getBoundingClientRect().right + gap : 8;
            const roomRight = rightCluster ? rightCluster.getBoundingClientRect().left - gap : window.innerWidth - 8;
            // The title bar slides up out of sight while it is tucked, and a rect
            // read then carries that slide, which opened the surface above the
            // window. Measure against where the bar rests instead: its layout box
            // (offsetTop ignores transforms) against where it is painted now.
            const titleBar = slot.closest<HTMLElement>('[data-titlebar]');
            const barShift = titleBar
                ? titleBar.getBoundingClientRect().top
                    - titleBar.offsetTop
                    - (titleBar.offsetParent?.getBoundingClientRect().top ?? 0)
                : 0;
            // Joined into the strip, the thing being filled is the STRIP, so the
            // geometry is the strip's pill and not the trigger's own box: same
            // top, same height, and `cover` is its full width. Anchoring to the
            // trigger instead filled the pill horizontally while sitting 4px
            // inside it top and bottom, which reads as a chip laid over the bar
            // rather than the bar being taken over.
            //
            // `width` is the box the surface STARTS from, so it still ends at
            // the trigger's right edge: the notification blooms out of the
            // trigger and runs to the end of the strip.
            //
            // Standalone (no strip on screen) there is nothing to fill: it keeps
            // its own box, `cover` is 0, and the surface grows from its centre.
            const pill = slot.closest('[data-nav-strip]') as HTMLElement | null;
            const pillR = pill?.getBoundingClientRect();
            if (pillR && pillR.width > 0) {
                setAnchor({
                    left: pillR.left, top: pillR.top - barShift,
                    width: Math.max(0, r.right - pillR.left),
                    height: pillR.height,
                    cover: pillR.width,
                    // Read off the strip rather than hard-coded, so a surface that
                    // covers it can never be a different shape from it. Two radii
                    // on the same box is what left the strip's corners visible
                    // around the notification filling it.
                    radius: parseFloat(getComputedStyle(pill!).borderTopLeftRadius) || 0,
                    roomLeft, roomRight,
                });
                return;
            }
            setAnchor({
                left: r.left, top: r.top - barShift,
                width: r.width || 72, height: r.height || 34,
                cover: 0,
                radius: 0,
                roomLeft, roomRight,
            });
        };
        measure();
        const ro = new ResizeObserver(measure);
        ro.observe(slot);
        if (slot.parentElement) ro.observe(slot.parentElement);
        // The strip changes width on its own: a Results tab appearing, the
        // search field focusing open. Both move the edge the surface fills to.
        const navEl = document.getElementById('sn-nav-slot');
        if (navEl) ro.observe(navEl);
        const pillEl = slot.closest('[data-nav-strip]');
        if (pillEl) ro.observe(pillEl);
        // When the left cluster outgrows its half of the bar it pushes the
        // centre right, so its width moves the slot too. Both clusters also
        // bound the room the surface may grow into.
        if (leftCluster) ro.observe(leftCluster);
        if (rightCluster) ro.observe(rightCluster);
        window.addEventListener('resize', measure);
        return () => {
            ro.disconnect();
            window.removeEventListener('resize', measure);
        };
    }, [slot]);

    const soundEnabled = settings.live_notifications?.play_sound ?? true;
    const notificationsEnabled = settings.live_notifications?.enabled ?? true;
    const showLiveNotifications = settings.live_notifications?.show_live_notifications ?? true;
    const showFavoriteLiveNotifications = settings.live_notifications?.show_favorite_live_notifications ?? true;
    const showUpdateNotifications = settings.live_notifications?.show_update_notifications ?? true;
    const showDropsNotifications = settings.live_notifications?.show_drops_notifications ?? true;
    const showFavoriteDropsNotifications = settings.live_notifications?.show_favorite_drops_notifications ?? true;
    const showChannelPointsNotifications = settings.live_notifications?.show_channel_points_notifications ?? true;
    const showBadgeNotifications = settings.live_notifications?.show_badge_notifications ?? true;
    const showGiftSubNotifications = settings.live_notifications?.show_gift_sub_notifications ?? true;
    const showTwitchRewardNotifications = settings.live_notifications?.show_twitch_reward_notifications ?? true;
    const useDynamicIsland = settings.live_notifications?.use_dynamic_island ?? true;
    const useToast = settings.live_notifications?.use_toast ?? true;
    // `quick_update_on_toast` used to be read here and used in no branch, while
    // its Settings row promised "clicking the update toast starts installing
    // right away". There is no update toast (removed deliberately, see
    // checkForUpdates), so the toggle configured nothing, and the row is gone.
    // The install action people actually needed lives in the changelog popup.

    // Save notifications to cache whenever they change
    useEffect(() => {
        saveCachedNotifications(notifications);
    }, [notifications]);

    // Mentions and replies: Rust's inbox for the signed-in account, newest
    // first. Rust owns the entries, their read state and the unread total; the
    // history draws the loaded pages and sends what the viewer does.
    const [mentions, setMentions] = useState<MentionEntry[]>([]);
    const [mentionsHaveMore, setMentionsHaveMore] = useState(false);
    const [mentionUnread, setMentionUnread] = useState(0);
    const mentionsLoaded = useRef(MENTION_PAGE);
    const mentionsLoadingOlder = useRef(false);
    useEffect(() => {
        mentionsLoaded.current = Math.max(MENTION_PAGE, mentions.length);
    }, [mentions.length]);
    const mutedRooms = settings.mentions?.muted_channels;
    const mutedRoomSet = useMemo(() => new Set((mutedRooms ?? []).map((r) => r.toLowerCase())), [mutedRooms]);

    // The first pages again, as many as are loaded, after every change Rust
    // reports (a new mention, a read, a removal, an account switch). Changes
    // come in bursts in a busy chat, so they are gathered for a moment.
    useEffect(() => {
        if (IS_MOBILE) return;
        let alive = true;
        let timer: ReturnType<typeof setTimeout> | null = null;
        const reload = () => {
            invoke<MentionPage>('get_mention_inbox', { limit: Math.min(mentionsLoaded.current, MENTION_PAGE_MAX) })
                .then((page) => {
                    if (!alive) return;
                    setMentions(page.entries);
                    setMentionsHaveMore(page.has_more);
                    setMentionUnread(page.unread_total);
                })
                .catch((e) => Logger.debug('[DynamicIsland] get_mention_inbox failed:', e));
        };
        reload();
        const unlisten = listen(MENTION_INBOX_CHANGED_EVENT, () => {
            if (timer) clearTimeout(timer);
            timer = setTimeout(reload, 120);
        });
        return () => {
            alive = false;
            if (timer) clearTimeout(timer);
            unlisten.then((fn) => fn());
        };
    }, []);

    const loadOlderMentions = useCallback(() => {
        const last = mentions[mentions.length - 1];
        if (!mentionsHaveMore || !last || mentionsLoadingOlder.current) return;
        mentionsLoadingOlder.current = true;
        invoke<MentionPage>('get_mention_inbox', { beforeMs: last.at_ms, limit: MENTION_PAGE })
            .then((page) => {
                setMentions((prev) => {
                    const seen = new Set(prev.map((m) => m.id));
                    return [...prev, ...page.entries.filter((m) => !seen.has(m.id))];
                });
                setMentionsHaveMore(page.has_more);
            })
            .catch((e) => Logger.debug('[DynamicIsland] older mentions failed:', e))
            .finally(() => { mentionsLoadingOlder.current = false; });
    }, [mentions, mentionsHaveMore]);

    // Read here at once; Rust confirms through the change event.
    const markMentionsReadHere = (ids: string[] | null) => {
        const hit = (m: MentionEntry) => !m.read && (ids === null || ids.includes(m.id));
        const newlyRead = mentions.filter(hit).length;
        if (ids === null) setMentionUnread(0);
        else if (newlyRead > 0) setMentionUnread((u) => Math.max(0, u - newlyRead));
        setMentions((prev) => prev.map((m) => (hit(m) ? { ...m, read: true } : m)));
    };

    // Opening one: Rust marks it read, its chat opens and scrolls to it.
    const openMention = (entry: MentionEntry) => {
        markMentionsReadHere([entry.id]);
        setIsExpanded(false);
        void openMentionInMain(entry.id, entry.open_key || entry.room);
    };

    const markMentionRead = (entry: MentionEntry, e: React.MouseEvent) => {
        e.stopPropagation();
        markMentionsReadHere([entry.id]);
        invoke('mark_mentions_read', { ids: [entry.id] })
            .catch((err) => Logger.debug('[DynamicIsland] mark_mentions_read failed:', err));
    };

    const removeMention = (entry: MentionEntry, e: React.MouseEvent) => {
        e.stopPropagation();
        if (!entry.read) setMentionUnread((u) => Math.max(0, u - 1));
        setMentions((prev) => prev.filter((m) => m.id !== entry.id));
        invoke('remove_mention', { id: entry.id })
            .catch((err) => Logger.debug('[DynamicIsland] remove_mention failed:', err));
    };

    const toggleMentionMute = (entry: MentionEntry, e: React.MouseEvent) => {
        e.stopPropagation();
        void toggleMentionChatMuted(entry.room);
    };

    // Size the preview pill to its content so long streamer/game names are shown
    // in full instead of being truncated. We measure the (non-wrapping) text's
    // natural width via scrollWidth and add the icon + padding chrome, clamped so
    // the pill never grows past the window. Runs before paint so the pill springs
    // straight to the right width. Anything beyond the max still truncates.
    useLayoutEffect(() => {
        if (!showPreview || !latestNotification || !previewTextRef.current) return;
        const textWidth = previewTextRef.current.scrollWidth;
        const CHROME = 72; // wrapper/content padding + leading icon + gap + breathing room
        const maxWidth = Math.min(560, window.innerWidth - 48);
        setPreviewWidth(Math.round(Math.max(120, Math.min(maxWidth, textWidth + CHROME))));
    }, [showPreview, latestNotification]);

    // Track window size for responsive notification center
    useEffect(() => {
        const handleResize = () => {
            setWindowSize({ width: window.innerWidth, height: window.innerHeight });
        };

        window.addEventListener('resize', handleResize);
        return () => window.removeEventListener('resize', handleResize);
    }, []);

    // Calculate responsive dimensions for notification center
    const getExpandedDimensions = () => {
        const { width, height } = windowSize;

        // Base dimensions
        let expandedWidth = 360;
        let maxHeight = 480;
        let itemHeight = 80;

        // Scale up for larger screens
        if (width >= 1920) {
            expandedWidth = Math.min(480, width * 0.25);
            maxHeight = Math.min(600, height * 0.6);
            itemHeight = 90;
        } else if (width >= 1440) {
            expandedWidth = Math.min(420, width * 0.28);
            maxHeight = Math.min(540, height * 0.55);
            itemHeight = 85;
        } else if (width >= 1280) {
            expandedWidth = Math.min(380, width * 0.3);
            maxHeight = Math.min(500, height * 0.5);
            itemHeight = 82;
        }

        return { expandedWidth, maxHeight, itemHeight };
    };

    const { expandedWidth, maxHeight, itemHeight } = getExpandedDimensions();

    // Check if a streamer is still live
    const checkIfStillLive = useCallback(async (userLogin: string): Promise<boolean> => {
        try {
            const streamData = await invoke('check_stream_online', { userLogin });
            return streamData !== null;
        } catch {
            return false;
        }
    }, []);

    // Play notification sound. Uses the shared Web-Audio engine (same one the
    // toast path uses) so the notification-center sound honors the user's chosen
    // Sound Style instead of a hardcoded tone, and reuses one AudioContext.
    const playNotificationSound = useCallback(() => {
        playGatedNotificationSound(settings.live_notifications?.sound_type, settings.live_notifications?.sound_volume);
    }, [settings.live_notifications?.sound_type, settings.live_notifications?.sound_volume]);

    // Every notification below is announced here once, in plain words, after
    // its settings gates passed. Rust shows it in a MultiChat window while this
    // window is hidden or minimized, and drops it otherwise; the sound stays
    // ours. A click on that card comes back as an activate event with the id,
    // and the action runs here, the window that can perform it.
    const mirroredActionsRef = useRef(new Map<string, () => void>());
    const mirrorNotification = useCallback((notice: {
        kind: MirroredNoticeKind;
        title: string;
        body?: string;
        imageUrl?: string | null;
        roundImage?: boolean;
        action?: { label: string; run: () => void };
        /** The action as data, for a Windows notification used after a restart. */
        target?: NoticeAction;
        /** A reply box on the Windows notification. */
        reply?: NoticeReply;
    }) => {
        if (IS_MOBILE) return;
        const id = `mirror-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
        const actions = mirroredActionsRef.current;
        if (notice.action) {
            actions.set(id, notice.action.run);
            // Only cards still on screen can be clicked, so a handful is plenty.
            while (actions.size > MAX_MIRRORED_ACTIONS) {
                actions.delete(actions.keys().next().value as string);
            }
        }
        const payload: MirroredNotice = {
            id,
            kind: notice.kind,
            title: notice.title,
            body: notice.body ?? '',
            imageUrl: notice.imageUrl || null,
            roundImage: !!notice.roundImage,
            actionLabel: notice.action?.label ?? null,
            action: notice.target ?? null,
            reply: notice.reply ?? null,
        };
        invoke<string | null>('mirror_notification', { notice: payload })
            .then((target) => { if (!target) actions.delete(id); })
            .catch((e) => {
                actions.delete(id);
                Logger.debug('[DynamicIsland] mirror_notification failed:', e);
            });
    }, []);

    // A notification used after a restart (or one Rust raised itself, like a
    // chat mention) leaves its action in Rust. It is taken once, on the event
    // or on mount when the click started the app, and run here.
    useEffect(() => {
        if (IS_MOBILE) return;
        const run = (action: NoticeAction) => {
            const store = useAppStore.getState();
            switch (action.type) {
                case 'watch': {
                    const go = () => { void useAppStore.getState().startStream(action.channel); };
                    if (!store.isBooting) { go(); return; }
                    const stop = useAppStore.subscribe((s) => {
                        if (!s.isBooting) { stop(); go(); }
                    });
                    return;
                }
                case 'whisper':
                    store.openWhisperWithUser({
                        id: action.userId,
                        login: action.login,
                        display_name: action.name,
                        profile_image_url: action.avatar ?? undefined,
                    });
                    return;
                case 'drops':
                    store.setShowDropsOverlay(true);
                    return;
                case 'badges':
                    store.setShowBadgesOverlay(true);
                    return;
                case 'changelog':
                    requestChangelog(action.version ?? undefined);
                    return;
                case 'rewards': {
                    // The chat showing that channel opens its wallet once it is up.
                    requestWallet(action.channel);
                    const current = store.currentStream?.user_login?.toLowerCase();
                    if (current === action.channel.toLowerCase()) return;
                    const go = () => { void useAppStore.getState().startStream(action.channel); };
                    if (!store.isBooting) { go(); return; }
                    const stop = useAppStore.subscribe((s) => {
                        if (!s.isBooting) { stop(); go(); }
                    });
                    return;
                }
                case 'settings':
                    if (SETTINGS_TABS_FROM_NOTICES.includes(action.tab as SettingsTab)) {
                        store.openSettings(action.tab as SettingsTab);
                    }
                    return;
                case 'mention':
                    // Marks it read, opens its chat (after boot when the click
                    // started the app) and leaves the jump for that chat.
                    void openMentionInMain(action.id, action.channel);
                    return;
            }
        };
        const take = () => {
            invoke<NoticeAction | null>('take_notice_action')
                .then((action) => { if (action) run(action); })
                .catch((e) => Logger.debug('[DynamicIsland] take_notice_action failed:', e));
        };
        take();
        const unlisten = listen(NOTICE_ACTION_PENDING_EVENT, take);
        return () => { unlisten.then((fn) => fn()); };
    }, []);

    useEffect(() => {
        const unlisten = listen<string>(MIRRORED_NOTIFICATION_ACTIVATE_EVENT, (event) => {
            const actions = mirroredActionsRef.current;
            const run = actions.get(event.payload);
            actions.delete(event.payload);
            run?.();
        });
        return () => { unlisten.then((fn) => fn()); };
    }, []);

    // Show the live preview for one item. A mention's preview is ONLY this:
    // its history row is the inbox entry Rust already filed.
    const showPreviewOf = useCallback((notification: IslandItem) => {
        setLatestNotification(notification);
        setShowPreview(true);

        // Hold the inline preview for the same time the matching toast would
        // stay up (live = 8s, everything else = 5s; mirrors AppStore.addToast),
        // then let the pill contract back to its idle size.
        if (previewTimeoutRef.current) {
            clearTimeout(previewTimeoutRef.current);
        }
        const previewHoldMs = notification.type === 'live' ? 8000 : 5000;
        previewTimeoutRef.current = setTimeout(() => {
            setShowPreview(false);
        }, previewHoldMs);
    }, []);

    // Add notification
    const addNotification = useCallback((notification: DynamicIslandNotification) => {
        setNotifications(prev => {
            const newNotifications = [notification, ...prev].slice(0, MAX_NOTIFICATIONS);
            return newNotifications;
        });
        showPreviewOf(notification);
    }, [showPreviewOf]);

    // A mention Rust decided to announce now. Its gate already checked the
    // setting, streamer mode, the per-chat cooldown, the message's age and
    // that the chat is not on screen; the island adds only its own switches.
    // No sound here: a mention or reply already rang the mention ping, and a
    // highlight phrase its own rule's sound.
    useEffect(() => {
        if (IS_MOBILE) return;
        const unlisten = listen<MentionEntry>(MENTION_NOTIFY_EVENT, (event) => {
            if (!notificationsEnabled || !useDynamicIsland) return;
            const entry = event.payload;
            if (!entry?.id) return;
            showPreviewOf(toMentionItem(entry));
        });
        return () => { unlisten.then((fn) => fn()); };
    }, [notificationsEnabled, useDynamicIsland, showPreviewOf]);

    // Mirror action-feedback toasts (fired from anywhere via AppStore.addToast)
    // into the notification center: these are notifications too, so they leave a
    // record even when the toast surface is muted. The store already gated the
    // emit on the island toggles; we re-check here so a mid-flight settings
    // change is honored. These are non-interactive log entries (clicking just
    // marks them read) and play no sound, since the action itself was the user's
    // own context.
    useEffect(() => {
        const unlisten = listen<{
            text: string;
            level?: SystemNotificationData['level'];
            avatarUrl?: string;
            avatarSevenTvId?: string;
            source?: SystemNotificationData['source'];
        }>('action-notification', (event) => {
            if (!notificationsEnabled || !useDynamicIsland) return;
            const { text, level, avatarUrl, avatarSevenTvId, source } = event.payload;
            if (!text) return;
            addNotification({
                id: `system-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
                type: 'system',
                timestamp: Date.now(),
                read: false,
                data: { title: '', message: text, level, avatar_url: avatarUrl, avatar_seventv_id: avatarSevenTvId, source } as SystemNotificationData,
            });
        });
        return () => { unlisten.then((fn) => fn()); };
    }, [addNotification, notificationsEnabled, useDynamicIsland]);

    // Updates: Rust checks shortly after start and every half hour
    // (services/update_watch.rs) and says when a version is new enough to
    // announce (once per version, across restarts). The title-bar indicator is
    // App's listener; this adds the notification-centre entry.
    useEffect(() => {
        const unlisten = listen<{ status: BundleUpdateStatus; announce: boolean }>('update://status', (event) => {
            const { status, announce } = event.payload;
            if (!announce || !notificationsEnabled || !showUpdateNotifications) return;
            if (useDynamicIsland) {
                addNotification({
                    id: `update-${status.latest_version}-${Date.now()}`,
                    type: 'update',
                    timestamp: Date.now(),
                    read: false,
                    data: {
                        current_version: status.current_version,
                        latest_version: status.latest_version,
                        has_update: true,
                    } as UpdateNotificationData,
                });
            }
            // No toast for available updates: the title-bar cog turning into a
            // download button is the quieter signal the user acts on.
            if (soundEnabled) {
                playNotificationSound();
            }
            mirrorNotification({
                kind: 'update',
                title: 'Update available',
                body: `StreamNook v${status.latest_version} is ready to download`,
                action: { label: 'See what\'s new', run: () => requestChangelog(status.latest_version) },
                target: { type: 'changelog', version: status.latest_version },
            });
        });
        return () => {
            unlisten.then((fn) => fn());
        };
    }, [notificationsEnabled, showUpdateNotifications, addNotification, soundEnabled, playNotificationSound, useDynamicIsland, mirrorNotification]);

    // Listen for live notifications
    useEffect(() => {
        const unlisten = listen<LiveNotificationFromBackend>('streamer-went-live', (event) => {
            const data = event.payload;

            // Two watchers emit this event now, and each has its own toggle:
            // the follow poller (no `source`) and the favorites sweep. Gating
            // both on `show_live_notifications` would mean turning off
            // notifications for your follows also silenced your favorites.
            if (!notificationsEnabled) return;
            const typeEnabled = data.source === 'favorite'
                ? showFavoriteLiveNotifications
                : showLiveNotifications;
            if (!typeEnabled) return;

            // Test notifications mirror whichever surfaces the user has enabled
            // so the "Test" button previews their actual setup:
            //  - Dynamic Island on -> drop a dummy entry in the notification
            //    center (auto-removed after a few seconds so tests don't pile up).
            //  - Toast on          -> ToastManager shows the decorated toast.
            // Sound plays exactly once: the toast path owns it when a toast is
            // shown, otherwise it plays here so the sound still previews with
            // toast popups off.
            if (data.is_test) {
                if (useDynamicIsland) {
                    const testId = data.notification_id;
                    addNotification({
                        id: testId,
                        type: 'live',
                        timestamp: Date.now(),
                        read: false,
                        data: {
                            streamer_name: data.streamer_name,
                            streamer_login: data.streamer_login,
                            streamer_avatar: data.streamer_avatar,
                            game_name: data.game_name,
                            game_image: data.game_image,
                            stream_title: data.stream_title,
                            is_live: true,
                            is_test: true,
                            providers: data.providers,
                        } as LiveNotificationData,
                    });
                    setTimeout(() => {
                        setNotifications(prev => prev.filter(n => n.id !== testId));
                    }, TEST_NOTIFICATION_TTL_MS);
                }

                if (useToast) {
                    emit('show-live-toast', data);
                } else if (soundEnabled) {
                    playNotificationSound();
                }
                return;
            }

            // Add to Dynamic Island if enabled (real notifications only)
            if (useDynamicIsland) {
                if (data.merge_into) {
                    // Same person, another platform. One row, two marks.
                    const mergeId = data.merge_into;
                    setNotifications(prev => prev.map(n =>
                        n.id === mergeId
                            ? {
                                ...n,
                                // Unread again: something new happened on it.
                                read: false,
                                data: { ...(n.data as LiveNotificationData), providers: data.providers },
                            }
                            : n,
                    ));
                } else {
                    addNotification({
                        id: data.notification_id,
                        type: 'live',
                        timestamp: Date.now(),
                        read: false,
                        data: {
                            streamer_name: data.streamer_name,
                            streamer_login: data.streamer_login,
                            streamer_avatar: data.streamer_avatar,
                            game_name: data.game_name,
                            game_image: data.game_image,
                            stream_title: data.stream_title,
                            is_live: true,
                            providers: data.providers,
                            seventv_id: data.seventv_id ?? undefined,
                        } as LiveNotificationData,
                    });
                }
            }

            // Show decorated toast if enabled - emit event for ToastManager to handle
            if (useToast) {
                emit('show-live-toast', data);
            }

            // Play the sound exactly once per event. When a toast is shown,
            // ToastManager plays it; otherwise the notification-center entry
            // covers it here. This keeps the two paths from doubling up.
            if (soundEnabled && useDynamicIsland && !useToast) {
                playNotificationSound();
            }

            // Another platform's go-live for an entry already announced is not
            // news; the card for the first one said it.
            if (!data.merge_into) {
                mirrorNotification({
                    kind: 'live',
                    title: `${data.streamer_name} is live`,
                    body: liveActivityText(data.game_name) || data.stream_title || '',
                    imageUrl: data.streamer_avatar,
                    roundImage: true,
                    action: { label: 'Watch', run: () => { void startStream(data.streamer_login); } },
                    target: { type: 'watch', channel: data.streamer_login },
                });
            }
        });

        return () => {
            unlisten.then((fn) => fn());
        };
    }, [addNotification, notificationsEnabled, showLiveNotifications, showFavoriteLiveNotifications, useDynamicIsland, useToast, addToast, startStream, soundEnabled, playNotificationSound, mirrorNotification]);

    // Listen for whisper notifications
    useEffect(() => {
        // Rust records each incoming whisper, then decides whether it is worth
        // a notification (services/whisper_inbox.rs: the master switch and the
        // Whispers type, streamer mode, and not the conversation on screen),
        // the same decision the phones post from. The announcement already
        // carries the sender's picture.
        const unlisten = listen<WhisperUpdate>('whisper-notify', (event) => {
            const whisper = event.payload.message;
            if (!whisper || whisper.is_sent) return;
            const data = {
                from_user_id: whisper.from_user_id,
                from_user_login: whisper.from_user_login,
                from_user_name: whisper.from_user_name,
                whisper_id: whisper.id,
                text: whisper.message,
            };
            const profileImageUrl = event.payload.meta.profile_image_url ?? undefined;

            // Add to Dynamic Island if enabled
            if (useDynamicIsland) {
                const notification: DynamicIslandNotification = {
                    id: `whisper-${data.whisper_id}`,
                    type: 'whisper',
                    timestamp: Date.now(),
                    read: false,
                    data: {
                        from_user_id: data.from_user_id,
                        from_user_login: data.from_user_login,
                        from_user_name: data.from_user_name,
                        message: data.text,
                        whisper_id: data.whisper_id,
                        profile_image_url: profileImageUrl,
                    } as WhisperNotificationData,
                };

                addNotification(notification);
            }

            // The sound follows its own toggle, not the surface: inside the
            // notification-centre branch it went quiet whenever the centre was off,
            // and the toast for these events never plays one.
            if (soundEnabled) {
                playNotificationSound();
            }

            // Show toast if enabled
            // Show toast if enabled
            if (useToast) {
                addToast(
                    `Whisper from ${data.from_user_name}: ${data.text.substring(0, 50)}${data.text.length > 50 ? '...' : ''}`,
                    'info',
                    {
                        label: 'Reply',
                        onClick: () => openWhisperWithUser({
                            id: data.from_user_id,
                            login: data.from_user_login,
                            display_name: data.from_user_name,
                            profile_image_url: profileImageUrl,
                        }),
                    },
                    { skipIsland: true }
                );
            }

            mirrorNotification({
                kind: 'whisper',
                title: `Whisper from ${data.from_user_name}`,
                body: data.text,
                imageUrl: profileImageUrl,
                roundImage: true,
                action: {
                    label: 'Open',
                    run: () => openWhisperWithUser({
                        id: data.from_user_id,
                        login: data.from_user_login,
                        display_name: data.from_user_name,
                        profile_image_url: profileImageUrl,
                    }),
                },
                target: {
                    type: 'whisper',
                    userId: data.from_user_id,
                    login: data.from_user_login,
                    name: data.from_user_name,
                    avatar: profileImageUrl ?? null,
                },
                reply: {
                    type: 'whisper',
                    userId: data.from_user_id,
                    login: data.from_user_login,
                    name: data.from_user_name,
                    avatar: profileImageUrl ?? null,
                },
            });
        });

        return () => {
            unlisten.then((fn) => fn());
        };
    }, [addNotification, playNotificationSound, soundEnabled, useDynamicIsland, useToast, addToast, openWhisperWithUser, mirrorNotification]);

    // Rewards Twitch names for your account (a badge earned, a drop reward
    // waiting), off its own notification feed, polled and classified in Rust.
    // These say WHICH reward, which the app's own drop and badge notices
    // cannot always do.
    useEffect(() => {
        const unlisten = listen<TwitchRewardFromBackend>('twitch-reward-received', (event) => {
            if (!notificationsEnabled || !showTwitchRewardNotifications) return;

            const data = event.payload;
            const title = data.kind === 'badge' ? 'Badge earned' : 'Drop reward';

            if (useDynamicIsland) {
                const notification: DynamicIslandNotification = {
                    id: `twitch-reward-${data.id}`,
                    // Twitch's timestamp, not arrival: the poll runs on an interval.
                    timestamp: Date.parse(data.created_at) || Date.now(),
                    type: 'twitch_reward',
                    read: false,
                    data: {
                        kind: data.kind,
                        body_plain: data.body_plain,
                        body_spans: data.body_spans,
                        thumbnail_url: data.thumbnail_url,
                        action_url: data.action_url ?? undefined,
                    } as TwitchRewardNotificationData,
                };

                addNotification(notification);
            }

            // The sound follows its own toggle, not the surface: inside the
            // notification-centre branch it went quiet whenever the centre was off,
            // and the toast for these events never plays one.
            if (soundEnabled) {
                playNotificationSound();
            }

            if (useToast) {
                addToast(data.body_plain, 'success', undefined, { skipIsland: true });
            }

            mirrorNotification({
                kind: 'twitch_reward',
                title,
                body: data.body_plain,
                imageUrl: data.thumbnail_url,
                action: {
                    label: 'View',
                    run: () => (data.kind === 'badge' ? setShowBadgesOverlay(true) : setShowDropsOverlay(true)),
                },
                target: { type: data.kind === 'badge' ? 'badges' : 'drops' },
            });
        });

        return () => {
            unlisten.then((fn) => fn());
        };
    }, [addNotification, playNotificationSound, notificationsEnabled, showTwitchRewardNotifications, soundEnabled, useDynamicIsland, useToast, addToast, setShowBadgesOverlay, setShowDropsOverlay, mirrorNotification]);

    // Gift subs you received. The only source for these is Twitch's own
    // notification feed, polled in Rust; the event arrives already filtered to
    // the gift_subscriptions category with its markdown split into spans.
    useEffect(() => {
        const unlisten = listen<GiftSubFromBackend>('gift-sub-received', (event) => {
            if (!notificationsEnabled || !showGiftSubNotifications) return;

            const data = event.payload;

            if (useDynamicIsland) {
                const notification: DynamicIslandNotification = {
                    id: `gift-sub-${data.id}`,
                    // Twitch's own timestamp, not arrival time: the poll runs on
                    // an interval, so Date.now() would bunch a gift received at
                    // 14:02 with one received at 14:06 under the same minute.
                    timestamp: Date.parse(data.created_at) || Date.now(),
                    type: 'gift_sub',
                    read: false,
                    data: {
                        body_plain: data.body_plain,
                        body_spans: data.body_spans,
                        thumbnail_url: data.thumbnail_url,
                        channel_login: data.channel_login ?? undefined,
                        action_url: data.action_url ?? undefined,
                    } as GiftSubNotificationData,
                };

                addNotification(notification);
            }

            // The sound follows its own toggle, not the surface: inside the
            // notification-centre branch it went quiet whenever the centre was off,
            // and the toast for these events never plays one.
            if (soundEnabled) {
                playNotificationSound();
            }

            if (useToast) {
                addToast(data.body_plain, 'success', undefined, { skipIsland: true });
            }

            const giftChannel = data.channel_login;
            mirrorNotification({
                kind: 'gift_sub',
                title: 'Gift subscription',
                body: data.body_plain,
                // The thumbnail is the gifter's avatar.
                imageUrl: data.thumbnail_url,
                roundImage: true,
                action: giftChannel
                    ? { label: 'Open channel', run: () => { void startStream(giftChannel); } }
                    : undefined,
                target: giftChannel ? { type: 'watch', channel: giftChannel } : undefined,
            });
        });

        return () => {
            unlisten.then((fn) => fn());
        };
    }, [addNotification, playNotificationSound, notificationsEnabled, showGiftSubNotifications, soundEnabled, useDynamicIsland, useToast, addToast, startStream, mirrorNotification]);

    // Somebody gave this member a StreamNook membership. Raised by Rust's
    // gift poll (services/membership_gifts.rs), once per gift, already worded.
    //
    // Gated on the master switch alone, with no per-type toggle: being given a
    // membership is rare and never unwanted, and a row nobody would turn off
    // does not earn a line in Settings.
    useEffect(() => {
        const unlisten = listen<MembershipGiftNotificationData>(
            'membership-gift-received',
            (event) => {
                if (!notificationsEnabled) return;
                const data = event.payload;

                if (useDynamicIsland) {
                    addNotification({
                        id: `membership-gift-${data.id ?? data.grantedAt}`,
                        type: 'membership_gift',
                        timestamp: Date.parse(data.grantedAt) || Date.now(),
                        read: false,
                        data,
                    });
                }
                if (soundEnabled) {
                    playNotificationSound();
                }

                if (useToast) {
                    addToast(
                        'You were given a StreamNook membership',
                        'success',
                        undefined,
                        { skipIsland: true, alwaysShow: true },
                    );
                }

                mirrorNotification({
                    kind: 'membership_gift',
                    title: 'You were given a StreamNook membership',
                    body: data.bodyPlain ?? 'Your perks are already active.',
                    action: { label: 'View', run: () => openSettings('Profile') },
                });
            },
        );
        return () => {
            unlisten.then((fn) => fn());
        };
    }, [addNotification, playNotificationSound, notificationsEnabled, soundEnabled, useDynamicIsland, useToast, addToast, openSettings, mirrorNotification]);

    // Listen for drop claimed notifications
    useEffect(() => {
        const unlisten = listen<DropClaimedEvent>('drop-claimed', (event) => {
            if (!notificationsEnabled || !showDropsNotifications) return;

            const data = event.payload;

            // Add to Dynamic Island if enabled
            if (useDynamicIsland) {
                const notification: DynamicIslandNotification = {
                    id: `drop-${Date.now()}-${data.drop_name}`,
                    type: 'drops',
                    timestamp: Date.now(),
                    read: false,
                    data: {
                        drop_name: data.drop_name,
                        game_name: data.game_name,
                        benefit_name: data.benefit_name,
                        benefit_image_url: data.benefit_image_url,
                    } as DropsNotificationData,
                };

                addNotification(notification);
            }

            // The sound follows its own toggle, not the surface: inside the
            // notification-centre branch it went quiet whenever the centre was off,
            // and the toast for these events never plays one.
            if (soundEnabled) {
                playNotificationSound();
            }

            // Show toast if enabled
            if (useToast) {
                addToast(
                    data.game_name
                        ? `Drop claimed: ${data.drop_name} (${data.game_name})`
                        : `Drop claimed: ${data.drop_name}`,
                    'success',
                    {
                        label: 'View',
                        onClick: () => setShowDropsOverlay(true),
                    },
                    { skipIsland: true }
                );
            }

            mirrorNotification({
                kind: 'drops',
                title: 'Drop claimed',
                body: data.game_name ? `${data.drop_name} (${data.game_name})` : data.drop_name,
                imageUrl: data.benefit_image_url,
                action: { label: 'View', run: () => setShowDropsOverlay(true) },
                target: { type: 'drops' },
            });
        });

        return () => {
            unlisten.then((fn) => fn());
        };
    }, [addNotification, notificationsEnabled, showDropsNotifications, useDynamicIsland, useToast, addToast, soundEnabled, playNotificationSound, setShowDropsOverlay, mirrorNotification]);

    // Channel points: Rust gathers a burst of earns and announces it once
    // (services/channel_points_summary.rs); this presents the summary.

    // Function to format reason codes into human-readable text
    const formatReasonCode = (reason: string): string => {
        const reasonMap: Record<string, string> = {
            'WATCH': 'watching',
            'WATCH_STREAK': 'watch streak',
            'CLAIM': 'bonus chest',
            'AUTOMATION': 'automation',
            'RAID': 'raid',
            'PREDICTION': 'prediction',
            'BITS': 'bits',
            'SUB': 'subscription',
            'GIFT_SUB': 'gift sub',
        };
        return reasonMap[reason.toUpperCase()] || reason.toLowerCase();
    };

    // Present one burst as a notification entry, a toast, or both.
    const showChannelPointsSummary = useCallback((summary: ChannelPointsSummary) => {
        const totalPoints = summary.total_points;
        const uniqueChannels = summary.channels.map(c => c.name);

        // Per-channel earned breakdown ("ninja +50, pokimane +82"), highest
        // first and capped so the line stays short. This names the actual
        // streamers the points came from instead of a bare "N channels" count.
        const MAX_CHANNELS_SHOWN = 3;
        const channelBreakdown = summary.channels.map(c => [c.name, c.points] as const);
        const channelListDisplay = channelBreakdown.length === 0
            ? null
            : channelBreakdown.length <= MAX_CHANNELS_SHOWN
                ? channelBreakdown.map(([name, pts]) => `${name} +${pts.toLocaleString()}`).join(', ')
                : `${channelBreakdown.slice(0, MAX_CHANNELS_SHOWN).map(([name, pts]) => `${name} +${pts.toLocaleString()}`).join(', ')} +${channelBreakdown.length - MAX_CHANNELS_SHOWN} more`;

        // What each reason gave, in words.
        const reasonCounts: Record<string, number> = {};
        summary.reasons.forEach(r => {
            const reason = formatReasonCode(r.name);
            reasonCounts[reason] = (reasonCounts[reason] || 0) + r.points;
        });
        const firstReason = formatReasonCode(summary.first_reason || 'watch');
        const lastBalance = summary.last_balance ?? undefined;
        // One channel: its face, and its own name for its points ("Goldwig").
        const only = summary.channels.length === 1 ? summary.channels[0] : undefined;
        const currency = only?.points_name || 'Channel Points';

        // Create notification data - we'll store the breakdown for expanded view
        const reasonSummary = Object.entries(reasonCounts)
            .map(([reason, points]) => `+${points.toLocaleString()} (${reason})`)
            .join(', ');

        // Determine channel name display - only use actual channel names, not reason summaries
        let channelNameDisplay: string | null = null;
        if (uniqueChannels.length === 1 && uniqueChannels[0]) {
            // Single channel - show its name
            channelNameDisplay = uniqueChannels[0];
        } else if (uniqueChannels.length > 1) {
            // Multiple channels - name the streamers + what each earned
            channelNameDisplay = channelListDisplay ?? `${uniqueChannels.length} channels`;
        }
        // If no channels, leave channelNameDisplay as null

        // Add to Dynamic Island if enabled
        if (useDynamicIsland) {
            const notification: DynamicIslandNotification = {
                id: `points-cluster-${Date.now()}`,
                type: 'channel_points',
                timestamp: Date.now(),
                read: false,
                data: {
                    // Store the channel name if available, otherwise show the reason summary as the "channel name" for display purposes
                    channel_name: channelNameDisplay || reasonSummary,
                    points_earned: totalPoints,
                    // Single channel: pass the balance so the line reads
                    // "streamer: N points". Multi-channel: omit it — the
                    // per-channel breakdown already carries each amount, and
                    // lastBalance is only the final channel's balance (a sum
                    // would be misleading).
                    total_points: uniqueChannels.length > 1 ? undefined : lastBalance,
                    // Mark if this is a reason summary (not a real channel name)
                    is_reason_summary: !channelNameDisplay,
                    avatar_url: only?.avatar_url,
                    points_name: only?.points_name,
                    icon_url: only?.icon_url,
                } as ChannelPointsNotificationData,
            };

            addNotification(notification);
        }

        // The sound follows its own toggle, not the surface: inside the
        // notification-centre branch it went quiet whenever the centre was off,
        // and the toast for these events never plays one.
        if (soundEnabled) {
            playNotificationSound();
        }

        // Show toast if enabled - show rich formatted version
        if (useToast) {
            const toastContent = (
                <div className="flex items-center gap-3 w-full">
                    {/* Free-standing, same as the notification centre. The toast is
                        already a surface; a filled disc inside it was a box in a box,
                        and ToastManager's own icons never had one. One channel
                        shows its face with its points icon attached instead. */}
                    {only?.avatar_url ? (
                        <PointsFace avatarUrl={only.avatar_url} iconUrl={only.icon_url} />
                    ) : (
                        <span className="sn-notif-glyph">
                            <PointsGlyph size={22} />
                        </span>
                    )}

                    {/* Text Content */}
                    <div className="flex-1 min-w-0">
                        <div className="text-base font-semibold text-textPrimary">
                            +{totalPoints.toLocaleString()} {currency}
                        </div>
                        <div className="text-xs text-textSecondary">
                            {uniqueChannels.length === 1 && uniqueChannels[0] ? (
                                // Single channel - show channel name, reason, and new balance
                                lastBalance
                                    ? `${uniqueChannels[0]} • ${firstReason} • ${lastBalance.toLocaleString()} ${only?.points_name || 'points'}`
                                    : `${uniqueChannels[0]} • ${firstReason}`
                            ) : uniqueChannels.length > 1 ? (
                                // Multiple channels - name the streamers + each earn
                                channelListDisplay ?? `From ${uniqueChannels.length} channels`
                            ) : (
                                // No channel info - show reason and balance if available
                                lastBalance
                                    ? `${firstReason} • ${lastBalance.toLocaleString()} points`
                                    : firstReason
                            )}
                        </div>
                    </div>
                </div>
            );

            addToast(toastContent, 'channel_points');
        }

        // No mirrorNotification here: Rust raises the system notification and
        // the MultiChat card for the same burst (services/points_notify.rs).
    }, [useDynamicIsland, useToast, addNotification, addToast, soundEnabled, playNotificationSound]);

    useEffect(() => {
        const unlisten = listen<ChannelPointsSummary>('channel-points-summary', (event) => {
            if (!notificationsEnabled || !showChannelPointsNotifications) return;
            if (event.payload.total_points > 0) showChannelPointsSummary(event.payload);
        });
        return () => {
            unlisten.then((fn) => fn());
        };
    }, [notificationsEnabled, showChannelPointsNotifications, showChannelPointsSummary]);

    // Listen for badge notifications from Rust backend
    useEffect(() => {
        if (!notificationsEnabled || !showBadgeNotifications) {
            return;
        }

        // Listen for badge-notification events from Rust
        const unlisten = listen<Array<{
            badge_name: string;
            badge_set_id: string;
            badge_version: string;
            badge_image_url: string;
            badge_description?: string;
            status: 'new' | 'available' | 'coming_soon';
            date_info?: string;
            enrichment?: Record<string, unknown>;
            window?: WindowRun[] | null;
        }>>('badge-notification', (event) => {
            const badges = event.payload;

            badges.forEach((badge) => {
                // Add to Dynamic Island if enabled
                if (useDynamicIsland) {
                    const notification: DynamicIslandNotification = {
                        id: `badge-${badge.badge_set_id}-${badge.badge_version}-${Date.now()}`,
                        type: 'badge',
                        timestamp: Date.now(),
                        read: false,
                        data: {
                            badge_name: badge.badge_name,
                            badge_set_id: badge.badge_set_id,
                            badge_version: badge.badge_version,
                            badge_image_url: badge.badge_image_url,
                            badge_description: badge.badge_description,
                            status: badge.status,
                            date_info: badge.date_info,
                            enrichment: badge.enrichment,
                            window: badge.window ?? null,
                        } as BadgeNotificationData,
                    };

                    addNotification(notification);
                }

                // The pushed status is a snapshot of when the relay sent it, so
                // a badge queued before its window opened would announce itself
                // as "Coming soon" after it had already gone live. Prefer the
                // window when the relay gave us one.
                // Rust resolves the window from the campaign dates, or from the
                // stamps in `date_info` when there are none.
                const derived = windowStatusAt(badge.window);
                const effectiveStatus = derived === 'available' ? 'available'
                    : derived === 'coming-soon' ? 'coming_soon'
                    : badge.status;
                // The relay sends UTC, so never surface the raw string.
                const when = formatBadgeDateInfo(badge.date_info);

                // Show toast if enabled
                if (useToast) {
                    const statusText = effectiveStatus === 'new' ? 'New badge' :
                        effectiveStatus === 'available' ? 'Now available' : 'Coming soon';

                    addToast(
                        `${statusText}: ${badge.badge_name}${when ? ` (${when})` : ''}`,
                        'info',
                        {
                            label: 'View',
                            onClick: () => setShowBadgesOverlay(true),
                        },
                        { skipIsland: true }
                    );
                }

                const statusLine = effectiveStatus === 'new' ? 'New badge available' :
                    effectiveStatus === 'available' ? 'Badge available now' : 'Badge coming soon';
                mirrorNotification({
                    kind: 'badge',
                    title: badge.badge_name,
                    body: `${statusLine}${when ? ` (${when})` : ''}`,
                    imageUrl: badge.badge_image_url,
                    action: { label: 'View', run: () => setShowBadgesOverlay(true) },
                    target: { type: 'badges' },
                });

                if (soundEnabled) {
                    playNotificationSound();
                }
            });
        });

        return () => {
            unlisten.then((fn) => fn());
        };
    }, [notificationsEnabled, showBadgeNotifications, useDynamicIsland, useToast, addNotification, addToast, soundEnabled, playNotificationSound, setShowBadgesOverlay, mirrorNotification]);

    // Listen for new drops in favorited categories (on app startup)
    useEffect(() => {
        if (!notificationsEnabled || !showFavoriteDropsNotifications) return;

        const unlisten = listen<{
            game_name: string;
            game_image: string;
            new_count: number;
            campaign_names: string[];
        }>('new-favorite-drops', (event) => {
            const data = event.payload;
            Logger.debug('[DynamicIsland] New drops in favorite category:', data);

            // Add to Dynamic Island if enabled
            if (useDynamicIsland) {
                const notification: DynamicIslandNotification = {
                    id: `drops-new-${Date.now()}-${data.game_name}`,
                    type: 'drops',
                    timestamp: Date.now(),
                    read: false,
                    data: {
                        drop_name: data.campaign_names[0] || 'New drops available',
                        game_name: data.game_name,
                        benefit_name: data.new_count > 1 
                            ? `${data.new_count} new campaigns` 
                            : data.campaign_names[0],
                        benefit_image_url: data.game_image,
                    },
                };

                addNotification(notification);
            }

            // Show toast notification
            if (useToast) {
                const countText = data.new_count === 1 
                    ? '1 new campaign' 
                    : `${data.new_count} new campaigns`;
                addToast(
                    `${data.game_name}: ${countText} available!`,
                    'info',
                    {
                        label: 'View',
                        onClick: () => setShowDropsOverlay(true),
                    },
                    { skipIsland: true }
                );
            }

            mirrorNotification({
                kind: 'drops',
                title: `New drops for ${data.game_name}`,
                body: data.new_count === 1
                    ? data.campaign_names[0]
                    : `${data.new_count} new campaigns available`,
                imageUrl: data.game_image,
                action: { label: 'View', run: () => setShowDropsOverlay(true) },
                target: { type: 'drops' },
            });

            if (soundEnabled) {
                playNotificationSound();
            }
        });

        return () => {
            unlisten.then((fn) => fn());
        };
    }, [notificationsEnabled, showFavoriteDropsNotifications, useDynamicIsland, useToast, addNotification, addToast, soundEnabled, playNotificationSound, setShowDropsOverlay, mirrorNotification]);

    // Click outside, or Esc, to close
    useEffect(() => {
        const handleClickOutside = (event: MouseEvent) => {
            // The trigger lives in the title bar now, outside islandRef, so
            // without this a click on it would collapse the surface in the same
            // gesture that opened it.
            if (triggerRef.current?.contains(event.target as Node)) return;
            if (islandRef.current && !islandRef.current.contains(event.target as Node)) {
                setIsExpanded(false);
            }
        };

        // Esc closes it too, unless the key belongs to a text field outside
        // the panel. Marked handled, so nothing else that answers Esc (leaving
        // the player's full screen) takes the same press.
        const handleKeyDown = (event: KeyboardEvent) => {
            if (event.key !== 'Escape' || event.defaultPrevented) return;
            const target = event.target;
            const inPanel = target instanceof Node && !!islandRef.current?.contains(target);
            if (
                !inPanel &&
                target instanceof HTMLElement &&
                (target.isContentEditable || !!target.closest('input, textarea, select'))
            ) {
                return;
            }
            event.preventDefault();
            setIsExpanded(false);
            // Keyboard focus inside the panel goes back to the door it opened from.
            if (islandRef.current?.contains(document.activeElement)) triggerRef.current?.focus();
        };

        if (isExpanded) {
            document.addEventListener('mousedown', handleClickOutside);
            document.addEventListener('keydown', handleKeyDown);
        }

        return () => {
            document.removeEventListener('mousedown', handleClickOutside);
            document.removeEventListener('keydown', handleKeyDown);
        };
    }, [isExpanded]);

    // Handle notification click
    const handleNotificationClick = async (notification: DynamicIslandNotification) => {
        // Mark as read
        setNotifications(prev =>
            prev.map(n => n.id === notification.id ? { ...n, read: true } : n)
        );

        if (notification.type === 'live') {
            const data = notification.data as LiveNotificationData;

            // Dummy "Test" entry — the mock login is a real channel, so clicking
            // it must not open a stream. Just dismiss the preview.
            if (data.is_test) {
                setNotifications(prev => prev.filter(n => n.id !== notification.id));
                return;
            }

            // Check if still live
            const isStillLive = await checkIfStillLive(data.streamer_login);

            if (isStillLive) {
                await startStream(data.streamer_login);
                setIsExpanded(false);
            } else {
                // Update notification to show offline
                setNotifications(prev =>
                    prev.map(n => {
                        if (n.id === notification.id && n.type === 'live') {
                            return {
                                ...n,
                                data: { ...(n.data as LiveNotificationData), is_live: false },
                            };
                        }
                        return n;
                    })
                );
            }
        } else if (notification.type === 'whisper') {
            const data = notification.data as WhisperNotificationData;

            // Open the WhispersWidget overlay with this user's conversation selected
            openWhisperWithUser({
                id: data.from_user_id,
                login: data.from_user_login,
                display_name: data.from_user_name,
                profile_image_url: data.profile_image_url,
            });

            setIsExpanded(false);
        } else if (notification.type === 'gift_sub') {
            const data = notification.data as GiftSubNotificationData;
            // channel_login is only set when the action URL was a plain channel
            // link, so this never tries to open "inventory" or "save-streak" as
            // a stream. Offline is fine; the app opens offline channels.
            if (data.channel_login) {
                await startStream(data.channel_login);
            }
            setIsExpanded(false);
        } else if (notification.type === 'twitch_reward') {
            const data = notification.data as TwitchRewardNotificationData;
            if (data.kind === 'badge') {
                setShowBadgesOverlay(true);
            } else {
                setShowDropsOverlay(true);
            }
            setIsExpanded(false);
        } else if (notification.type === 'membership_gift') {
            openSettings('Profile');
            setIsExpanded(false);
        } else if (notification.type === 'update') {
            // The changelog on the NEW version's notes, with Install beside
            // them, so the reason to update arrives before the update does.
            requestChangelog(useAppStore.getState().updateInfo?.latest_version);
            setIsExpanded(false);
        } else if (notification.type === 'drops' || notification.type === 'channel_points') {
            // Open drops overlay
            setShowDropsOverlay(true);
            setIsExpanded(false);
        } else if (notification.type === 'badge') {
            // Open badges overlay
            setShowBadgesOverlay(true);
            setIsExpanded(false);
        }
    };

    // Clear notification
    const clearNotification = (id: string, e: React.MouseEvent) => {
        e.stopPropagation();
        setNotifications(prev => prev.filter(n => n.id !== id));
    };

    // Clear all notifications, the mentions with them: the button clears what
    // the panel shows.
    const clearAllNotifications = () => {
        setNotifications([]);
        if (mentions.length > 0 || mentionUnread > 0) {
            setMentions([]);
            setMentionsHaveMore(false);
            setMentionUnread(0);
            invoke('clear_mention_inbox')
                .catch((e) => Logger.debug('[DynamicIsland] clear_mention_inbox failed:', e));
        }
    };

    // Mark single notification as read
    const markAsRead = (id: string, e: React.MouseEvent) => {
        e.stopPropagation();
        setNotifications(prev =>
            prev.map(n => n.id === id ? { ...n, read: true } : n)
        );
    };

    // Mark all notifications as read
    const markAllAsRead = () => {
        setNotifications(prev =>
            prev.map(n => ({ ...n, read: true }))
        );
        if (mentionUnread > 0) {
            markMentionsReadHere(null);
            invoke('mark_mentions_read', {})
                .catch((e) => Logger.debug('[DynamicIsland] mark_mentions_read failed:', e));
        }
    };

    // Get unread count: the island's own, plus every unread mention Rust
    // holds (loaded or not).
    const unreadCount = notifications.filter(n => !n.read).length + mentionUnread;

    // One history, newest first. While older mentions are still unloaded the
    // island's own entries stop at the oldest loaded mention, so loading the
    // next page only ever adds below the end and never pushes rows down.
    const items = useMemo<IslandItem[]>(() => {
        const cutoff = mentionsHaveMore && mentions.length > 0
            ? mentions[mentions.length - 1].at_ms
            : -Infinity;
        return [
            ...notifications.filter((n) => n.timestamp >= cutoff),
            ...mentions.map(toMentionItem),
        ].sort((a, b) => b.timestamp - a.timestamp);
    }, [notifications, mentions, mentionsHaveMore]);
    // A platform mark beside the chat only where mentions from more than one
    // platform share the list.
    const mentionPlatformsMix = useMemo(() => new Set(mentions.map((m) => m.platform)).size > 1, [mentions]);


    // Format time ago - uses 'now' state (updated every 30s) to avoid impure function calls during render
    // Day buckets for the list headers. Calendar days rather than rolling
    // windows: "Today" has to mean today, or a header is worse than none.
    const groupLabel = (timestamp: number) => {
        const d = new Date(timestamp);
        const now = new Date();
        const sameDay = (a: Date, b: Date) =>
            a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
        if (sameDay(d, now)) return 'Today';
        const yesterday = new Date(now);
        yesterday.setDate(now.getDate() - 1);
        if (sameDay(d, yesterday)) return 'Yesterday';
        return 'Earlier';
    };

    const formatTimeAgo = (timestamp: number) => {
        const seconds = Math.floor((now - timestamp) / 1000);
        if (seconds < 60) return 'Just now';
        const minutes = Math.floor(seconds / 60);
        if (minutes < 60) return `${minutes}m ago`;
        const hours = Math.floor(minutes / 60);
        if (hours < 24) return `${hours}h ago`;
        const days = Math.floor(hours / 24);
        return `${days}d ago`;
    };

    // Calculate collapsed width based on notifications
    const getCollapsedWidth = () => {
        // Joined into the strip, a preview CONSUMES it: the notification takes
        // the whole nav pill and hands it back, which is the same move the
        // search makes from the other end. Fitting the pill to the message
        // instead leaves it stopping at some arbitrary point mid-strip.
        if (anchor?.cover) return anchor.cover;
        if (showPreview && latestNotification) {
            // Fit the pill to the current preview's content (measured in the
            // useLayoutEffect above) so the whole message shows instead of
            // truncating. The pill is center-anchored (left-1/2 +
            // -translate-x-1/2), so it expands outward from the middle on both
            // sides, then contracts back once the preview auto-hides.
            return previewWidth;
        }
        // Idle never renders any more -- the trigger in the title bar IS the
        // resting state -- so this only backstops a preview with no measurement
        // yet. Matching the trigger's own width keeps the growth starting from
        // exactly its edge rather than jumping.
        return 72;
    };

    // Does the black surface have anything to say? Idle it is invisible and the
    // trigger in the title bar stands in its place.
    // Inside the strip it is a strip member: flat, like the tab pills beside it,
    // because a second blur of an already-blurred panel is visually identical
    // and costs a nested compositing layer. Standalone it is chrome in its own
    // right and keeps the full glaze.
    const inline = slot?.id === 'sn-island-slot-inline';

    const wantsSurface = isExpanded || (showPreview && !!latestNotification);
    // ...but it has to stay on screen for the whole collapse, or the shrink
    // never plays: hiding on the same tick the state flips means the bar simply
    // vanishes at full width. `surfaceShown` therefore turns on with the
    // request and off only when the spring has finished putting it back.
    const [surfaceShown, setSurfaceShown] = useState(false);
    useEffect(() => {
        if (wantsSurface) {
            setSurfaceShown(true);
            return;
        }
        // 340ms: just past the end of the container's fade below.
        //
        // This used to hang off the spring's `onAnimationComplete`, and THAT is
        // what produced the pause. A spring is not finished when it looks
        // finished: it arrives, then runs a long invisible tail until framer's
        // rest thresholds are met. So the surface sat there, fully opaque and
        // covering the notification area, waiting for an event nobody could
        // see, and then blinked out. A fixed clock cannot do that.
        const t = setTimeout(() => setSurfaceShown(false), 340);
        return () => clearTimeout(t);
    }, [wantsSurface]);
    // In the bar row the surface never runs over the title bar's icon
    // clusters. Covering the strip it already has the strip's bounds.
    // Standalone it grows from the door's centre, so its width is capped at the
    // room between the clusters, and when the door sits off-centre in that room
    // (a wide left cluster pushes it right) the surface slides back inside
    // rather than shrinking to twice the short side. Open as a list where that
    // room is too narrow to read (a narrow window, Compact View), it drops below
    // the bar row at a usable width instead (utils/islandGeometry.ts).
    const surfaceTargetWidth = isExpanded
        ? (anchor?.cover || expandedWidth)
        : wantsSurface
            ? getCollapsedWidth()
            : (anchor?.width ?? 72);
    const {
        width: surfaceWidth,
        shift: surfaceShift,
        drop: surfaceDrop,
        maxHeight: surfaceMaxHeight,
    } = islandSurfaceGeometry({
        anchor,
        targetWidth: surfaceTargetWidth,
        expanded: isExpanded,
        panelWidth: expandedWidth,
        viewportWidth: windowSize.width,
        viewportHeight: windowSize.height,
    });

    const shown = items.slice(0, 3);
    const hasSeveralDays = new Set(items.map((n) => groupLabel(n.timestamp))).size > 1;
    const overflow = items.length - shown.length;

    return (
        <>
            {slot && createPortal(
                /* The door, not the surface. It stays put and stays small; the
                   surface above grows out of it and collapses back into it. */
                <Tooltip content="Notifications" side="bottom">
                    <button
                        ref={triggerRef}
                        type="button"
                        aria-label={unreadCount > 0 ? `Notifications, ${unreadCount} unread` : 'Notifications'}
                        aria-expanded={isExpanded}
                        data-tauri-drag-region="false"
                        onClick={() => setIsExpanded((v) => !v)}
                        className={
                            inline
                                // Bare inside the strip. The strip already carries a
                                // pill, the one that slides between the tabs, and a
                                // second one sitting next to it reads as two
                                // competing selections rather than one control and
                                // one indicator. The hairline is what separates it.
                                ? 'sn-notif-trigger sn-notif-trigger--inline'
                                // Standalone it IS the chrome, so it takes the full
                                // material like the clusters either side of it.
                                : 'sn-notif-trigger chrome-glaze chrome-glaze--control'
                        }
                    >
                        {shown.length === 0 ? (
                            <Bell size={15} className="text-textSecondary" />
                        ) : (
                            <>
                                {shown.map((n, i) => (
                                    <span
                                        key={n.id}
                                        // Dim the read, like the rows: an unread
                                        // face at full strength is the island's
                                        // "something new" at rest.
                                        className={n.read ? 'sn-notif-face sn-notif-face--read' : 'sn-notif-face'}
                                        /* Overlapped rather than spaced: a stack reads as
                                           "several things" at a glance and stays narrow,
                                           where discrete slots got wide fast and needed
                                           dividers that fought the glaze's own rim. */
                                        // The newest on top: it is the one the
                                        // stack is announcing, and a mark (unlike
                                        // a face) is unreadable half covered.
                                        style={{ marginLeft: i ? -4 : undefined, zIndex: shown.length - i }}
                                    >
                                        {renderPreviewIcon(n)}
                                    </span>
                                ))}
                                {overflow > 0 && <span className="sn-notif-more">+{overflow}</span>}
                            </>
                        )}
                    </button>
                </Tooltip>,
                slot,
            )}
            <div
                ref={islandRef}
                // Rendered at the app root (see App.tsx) and pinned to the top
                // center, so it is NOT trapped inside the title bar's stacking
                // context. While Settings is open we lift it to z-[55] so the pill
                // floats just above the settings blur overlay (z-50) and its
                // notification/preview animation stays visible; otherwise it sits
                // at the normal title-bar layer (z-50). The element stays mounted
                // across this toggle (only the class changes), so there is no
                // re-mount flash of the unread-count badge.
                // Pinned to the trigger's LEFT edge rather than recentred, so the
                // surface grows rightward over the nav and gives it back on
                // collapse. That also retires the recentring this file blames for
                // the settle jitter: only `width` animates now, nothing is
                // fighting a translateX(-50%).
                //
                // Still rendered at the app root (see App.tsx) so it is not
                // trapped in the title bar's stacking context, and still lifted
                // to z-[55] over the settings blur overlay.
                className={`fixed ${isSettingsOpen ? 'z-[55]' : 'z-50'}`}
                // While the bar is lifted over the player the surface takes the
                // bar's menu layer, above the door it grows from. Over the
                // player's full screen only the open panel does (previews stay
                // under the video), and anything in it that navigates leaves
                // full screen first, like the bar itself.
                onClickCapture={isPlayerFullscreen ? (e) => exitPlayerFullscreenForBarAction(e.target) : undefined}
                style={{
                    zIndex: barLifted && (isExpanded || !isPlayerFullscreen) ? 'var(--sn-z-titlebar-menu)' : undefined,
                    // Anchored to the trigger's LEFT edge only when there is a
                    // nav to the right of it to take over: that is the whole
                    // point of growing that way. In a stream view Home is
                    // unmounted, so nothing is portaled into the nav slot, and
                    // opening rightward threw a panel out across empty bar for
                    // no reason. With nothing to cover it reverts to what a
                    // dynamic island normally does and grows from its own centre.
                    left: anchor
                        ? (anchor.cover ? anchor.left : anchor.left + anchor.width / 2)
                        : '50%',
                    top: anchor ? anchor.top : 6,
                    // No anchor means no title bar (mobile): keep the old centring.
                    transform: anchor && anchor.cover ? undefined : 'translateX(-50%)',
                    // Idle is invisible, and an invisible pill must not eat the
                    // clicks meant for the nav underneath it.
                    //
                    // No fade. The surface appears at exactly the trigger's box,
                    // pitch black and opaque, so the swap is invisible and the
                    // only thing the eye can follow is the size springing out:
                    // the button becomes the bar. Cross-fading instead was the
                    // whole problem, because for 180ms you saw the trigger
                    // THROUGH a half-transparent black pill, which reads as a
                    // panel appearing rather than a control opening.
                    opacity: surfaceShown && wantsSurface ? 1 : 0,
                    pointerEvents: wantsSurface ? 'auto' : 'none',
                    // Opening: instant, so the swap at the trigger's box stays
                    // invisible and the spring is the only thing you follow.
                    //
                    // Closing: hold at full opacity for 180ms while the spring
                    // does the collapse you already liked, THEN fade over 120ms.
                    // 180ms is about where this spring is visually home (natural
                    // frequency 20 rad/s, damping ratio 0.89), so the fade lands
                    // on the tail rather than replacing the travel.
                    transition: wantsSurface ? 'none' : 'opacity 120ms linear 180ms',
                    WebkitAppRegion: 'no-drag',
                } as React.CSSProperties}
            >
                <motion.div
                    // No `layout` here on purpose: width/height are animated
                    // explicitly below, and the parent centers with translateX(-50%).
                    // Framer's layout projection fighting that recentering is what
                    // caused the left/right jitter as the pill settled back.
                    initial={false}
                    animate={{
                        // Idle is the trigger's own box, so the surface visibly
                        // shrinks back INTO it rather than blinking out.
                        width: surfaceWidth,
                        // Only ever non-zero standalone, where the surface is
                        // held inside the room between the clusters, or for a
                        // list that opens below the bar row. The same spring as
                        // the width, so they travel as one.
                        x: surfaceShift,
                        y: surfaceDrop,
                        // Collapsed pill sits centered in the title bar. The bar is
                        // h-[40px] (less a 1px bottom border); container at top-1.5 (6px)
                        // + a 28px pill leaves an even gap above and below.
                        height: isExpanded
                            // Empty, it still has to fit the empty state under
                            // the header, or the panel opens as a bare header.
                            ? Math.min(maxHeight, surfaceMaxHeight ?? Infinity, items.length > 0 ? 64 + items.length * itemHeight : EMPTY_PANEL_HEIGHT)
                            : (anchor?.height ?? 28),
                    }}
                    // One spring, both directions. The collapse was never the
                    // problem and every attempt to "improve" it (a tween, a
                    // retimed tween) only replaced something that already worked.
                    transition={{
                        type: 'spring',
                        stiffness: 360,
                        damping: 32,
                        mass: 0.9,
                    }}
                    onClick={() => {
                        if (!isExpanded) {
                            setShowPreview(false);
                            if (previewTimeoutRef.current) {
                                clearTimeout(previewTimeoutRef.current);
                            }
                            // A mention's preview goes straight to the message,
                            // like its notification and its row.
                            if (showPreview && latestNotification?.type === 'mention') {
                                openMention(latestNotification.data);
                                return;
                            }
                            setIsExpanded(true);
                        }
                    }}
                    className="dynamic-island chrome-glaze chrome-glaze--frosted overflow-hidden cursor-pointer"
                    style={{
                        backgroundColor: '#000000',
                        // Capsule when small, so it matches the trigger it grows
                        // out of; softer rectangle once it is a panel.
                        // Open as a list it is a panel and takes a panel's corner.
                        // Sitting in the strip it takes the STRIP's corner, measured,
                        // so it lands exactly on top of it.
                        borderRadius: isExpanded
                            ? 20
                            : (anchor?.radius || (anchor ? anchor.height / 2 : 14)),
                        // No outer rim. The expanded panel keeps a soft black drop
                        // shadow for depth; the collapsed pill is pure black. Unread
                        // still surfaces via the accent dot, not a border ring.
                        boxShadow: isExpanded
                            ? '0 8px 32px rgba(0, 0, 0, 0.4)'
                            : 'none',
                        transition: 'box-shadow 0.3s ease',
                    }}
                >
                    {/* Collapsed State */}
                    <AnimatePresence mode="wait">
                        {!isExpanded && (
                            <motion.div
                                initial={{ opacity: 0 }}
                                animate={{ opacity: 1 }}
                                exit={{ opacity: 0 }}
                                transition={{ duration: 0.15 }}
                                className="flex items-center h-full px-2"
                            >
                                {showPreview && latestNotification ? (
                                    // Preview latest notification
                                    <motion.div
                                        className="flex items-center justify-center gap-2 w-full px-3"
                                        initial={{ opacity: 0 }}
                                        animate={{ opacity: 1 }}
                                        transition={{ duration: 0.2 }}
                                    >
                                        {renderPreviewIcon(latestNotification)}
                                        <span ref={previewTextRef} className="text-white text-[11px] font-medium truncate min-w-0">
                                            {getPreviewText(latestNotification)}
                                            {/* A mention also says what was said, quieter, after its line. */}
                                            {latestNotification.type === 'mention' && !latestNotification.data.removed && !!latestNotification.data.text && (
                                                <span className="ml-1.5 font-normal text-white/55">{latestNotification.data.text}</span>
                                            )}
                                        </span>
                                    </motion.div>
                                ) : null}
                            </motion.div>
                        )}
                    </AnimatePresence>

                    {/* Expanded State */}
                    <AnimatePresence>
                        {isExpanded && (
                            <motion.div
                                initial={{ opacity: 0 }}
                                animate={{ opacity: 1 }}
                                exit={{ opacity: 0 }}
                                transition={{ duration: 0.15 }}
                                className="flex flex-col h-full"
                            >
                                {/* Header.
                                    Actions live up here now, not under the list.
                                    Below the list they scrolled away the moment
                                    there was anything to clear, so the one control
                                    you wanted was the one you had to scroll to
                                    reach. This row does not scroll.

                                    The sound indicator only appears when sound is
                                    OFF. A speaker icon shown while sound is on
                                    reports the absence of a problem, which is the
                                    default state and therefore nothing to say. */}
                                <div
                                    className="sn-notif-head relative flex flex-shrink-0 items-center gap-2 px-4"
                                    onClick={(e) => {
                                        // The header background closes the panel.
                                        // This replaces an invisible strip across
                                        // the middle third, which nothing could
                                        // have told you was there. Buttons stop
                                        // propagation, so only the empty run
                                        // between them acts.
                                        e.stopPropagation();
                                        setIsExpanded(false);
                                    }}
                                >
                                    <span className="text-[13px] font-semibold tracking-tight text-white">
                                        Notifications
                                    </span>
                                    {unreadCount > 0 && (
                                        <span className="sn-notif-count">{unreadCount}</span>
                                    )}
                                    {!soundEnabled && (
                                        <Tooltip content="Notification sounds are off" side="bottom">
                                            <SpeakerSlash size={14} className="text-white/35" />
                                        </Tooltip>
                                    )}

                                    <span className="flex-1" />

                                    {unreadCount > 0 && (
                                        <Tooltip content="Mark all read" side="bottom">
                                            <button
                                                type="button"
                                                onClick={(e) => {
                                                    e.stopPropagation();
                                                    markAllAsRead();
                                                }}
                                                className="sn-notif-action"
                                                aria-label="Mark all read"
                                                // Changes nothing on screen behind the
                                                // panel, so it keeps the player's full screen.
                                                data-keeps-player-fullscreen
                                            >
                                                <CheckCheck size={15} />
                                            </button>
                                        </Tooltip>
                                    )}
                                    {items.length > 0 && (
                                        <Tooltip content="Clear all" side="bottom">
                                            <button
                                                type="button"
                                                onClick={(e) => {
                                                    e.stopPropagation();
                                                    clearAllNotifications();
                                                }}
                                                className="sn-notif-action sn-notif-action--danger"
                                                aria-label="Clear all"
                                                data-keeps-player-fullscreen
                                            >
                                                <Trash size={15} />
                                            </button>
                                        </Tooltip>
                                    )}
                                </div>

                                {/* Notifications List */}
                                <div
                                    // Rows running past the rim fade out rather
                                    // than being cut; the list's bottom padding
                                    // is what sits under the fade at the end.
                                    className="flex-1 overflow-y-auto scrollbar-thin [mask-image:linear-gradient(to_bottom,#000_calc(100%-20px),transparent)]"
                                    onScroll={(e) => {
                                        const el = e.currentTarget;
                                        if (el.scrollHeight - el.scrollTop - el.clientHeight < 240) loadOlderMentions();
                                    }}
                                >
                                    {items.length === 0 ? (
                                        <div className="flex flex-col items-center justify-center gap-3 py-14">
                                            <span className="sn-notif-empty-ring">
                                                <Bell size={20} className="text-textSecondary" />
                                            </span>
                                            <span className="text-[13px] font-medium text-textSecondary">
                                                Nothing to catch up on
                                            </span>
                                        </div>
                                    ) : (
                                        <div className="pb-4">
                                            {items.map((notification, i) => {
                                            // A single "Today" over a list that is
                                            // entirely today is a label telling you
                                            // nothing, so headers only appear once
                                            // there is more than one bucket to tell
                                            // apart.
                                            const label = groupLabel(notification.timestamp);
                                            const showHeader = hasSeveralDays
                                                && (i === 0 || groupLabel(items[i - 1].timestamp) !== label);
                                            const open = () => {
                                                if (notification.type === 'mention') openMention(notification.data);
                                                else void handleNotificationClick(notification);
                                            };
                                            return (
                                            <Fragment key={notification.id}>
                                            {showHeader && <div className="sn-notif-group">{label}</div>}
                                                <motion.div
                                                    layout
                                                    initial={{ opacity: 0, y: -10 }}
                                                    animate={{ opacity: 1, y: 0 }}
                                                    exit={{ opacity: 0, x: -100 }}
                                                    onClick={open}
                                                    // A row is a control: it opens what it
                                                    // announces. As a button it is reachable
                                                    // by keyboard, and a click on it leaves
                                                    // the player's full screen first.
                                                    role="button"
                                                    tabIndex={0}
                                                    onKeyDown={(e) => {
                                                        if (e.key !== 'Enter' && e.key !== ' ') return;
                                                        if (e.target !== e.currentTarget) return;
                                                        e.preventDefault();
                                                        open();
                                                    }}
                                                    className={`sn-notif-row group ${
                                                        notification.read ? '' : 'sn-notif-row--unread'
                                                    }`}
                                                >
                                                    {notification.type === 'mention' ? (
                                                        <MentionRowBody
                                                            entry={notification.data}
                                                            showPlatform={mentionPlatformsMix}
                                                        />
                                                    ) : notification.type === 'live' ? (
                                                        <>
                                                            {/* Live notification */}
                                                            <div className="relative flex-shrink-0">
                                                                {(notification.data as LiveNotificationData).streamer_avatar ? (
                                                                    <SevenTvAvatar
                                                                        cosmeticsId={(notification.data as LiveNotificationData).seventv_id}
                                                                        px={34}
                                                                        motion="hover"
                                                                        src={(notification.data as LiveNotificationData).streamer_avatar}
                                                                        alt=""
                                                                        className="w-[34px] h-[34px] rounded-full object-cover"
                                                                    />
                                                                ) : (
                                                                    <div className="sn-notif-glyph">
                                                                        <User size={21} className="text-white/45" />
                                                                    </div>
                                                                )}
                                                                {(notification.data as LiveNotificationData).is_live && (
                                                                    <div className="absolute -bottom-0.5 -right-0.5 w-3.5 h-3.5 bg-red-500 rounded-full border-2 border-black" />
                                                                )}
                                                            </div>
                                                            <div className="flex-1 min-w-0">
                                                                <div className="flex items-center gap-2">
                                                                    <Radio size={14} className="text-red-500 flex-shrink-0" />
                                                                    <span className="text-white text-sm font-semibold truncate">
                                                                        {(notification.data as LiveNotificationData).streamer_name}
                                                                    </span>
                                                                    {/* The marks carry the platform. With one they
                                                                        say where the click goes; with two they are
                                                                        the whole reason the row is one row. */}
                                                                    {!!(notification.data as LiveNotificationData).providers?.length && (
                                                                        <span className="sn-notif-marks">
                                                                            {(notification.data as LiveNotificationData).providers!.map((pid) => (
                                                                                <ProviderIcon key={pid} provider={pid} size="12px" />
                                                                            ))}
                                                                        </span>
                                                                    )}
                                                                </div>
                                                                <p className="text-white/50 text-sm truncate mt-0.5">
                                                                    {(notification.data as LiveNotificationData).is_live
                                                                        ? (notification.data as LiveNotificationData).game_name || 'Streaming'
                                                                        : 'Offline'
                                                                    }
                                                                </p>
                                                            </div>
                                                        </>
                                                    ) : notification.type === 'whisper' ? (
                                                        <>
                                                            {/* Whisper notification */}
                                                            <div className="relative flex-shrink-0">
                                                                {(notification.data as WhisperNotificationData).profile_image_url ? (
                                                                    <SevenTvAvatar
                                                                        cosmeticsId={(notification.data as WhisperNotificationData).from_user_id}
                                                                        px={34}
                                                                        motion="hover"
                                                                        src={(notification.data as WhisperNotificationData).profile_image_url}
                                                                        alt=""
                                                                        className="w-[34px] h-[34px] rounded-full object-cover"
                                                                    />
                                                                ) : (
                                                                    <div className="sn-notif-glyph">
                                                                        <MessageCircle size={21} className="text-purple-400" />
                                                                    </div>
                                                                )}
                                                            </div>
                                                            <div className="flex-1 min-w-0">
                                                                <div className="flex items-center gap-2">
                                                                    <MessageCircle size={14} className="text-purple-400 flex-shrink-0" />
                                                                    <span className="text-white text-sm font-semibold truncate">
                                                                        {(notification.data as WhisperNotificationData).from_user_name}
                                                                    </span>
                                                                </div>
                                                                <p className="text-white/50 text-sm truncate mt-0.5">
                                                                    {(notification.data as WhisperNotificationData).message}
                                                                </p>
                                                            </div>
                                                        </>
                                                    ) : notification.type === 'gift_sub' ? (
                                                        <>
                                                            {/* Gift sub received */}
                                                            <div className="relative flex-shrink-0">
                                                                {(notification.data as GiftSubNotificationData).thumbnail_url ? (
                                                                    <img
                                                                        src={(notification.data as GiftSubNotificationData).thumbnail_url}
                                                                        alt=""
                                                                        className="w-[34px] h-[34px] rounded-full object-cover"
                                                                    />
                                                                ) : (
                                                                    <div className="sn-notif-glyph">
                                                                        <Gift size={21} className="text-pink-400" />
                                                                    </div>
                                                                )}
                                                            </div>
                                                            <div className="flex-1 min-w-0">
                                                                <div className="flex items-center gap-2">
                                                                    <Gift size={14} className="text-pink-400 flex-shrink-0" />
                                                                    <span className="text-white text-sm font-semibold truncate">
                                                                        Gift subscription
                                                                    </span>
                                                                </div>
                                                                <p className="text-white/50 text-sm truncate mt-0.5">
                                                                    {(notification.data as GiftSubNotificationData).body_spans.map((span, i) => (
                                                                        <span key={i} className={span.bold ? 'text-white/80 font-semibold' : undefined}>
                                                                            {span.text}
                                                                        </span>
                                                                    ))}
                                                                </p>
                                                            </div>
                                                        </>
                                                    ) : notification.type === 'twitch_reward' ? (
                                                        <>
                                                            {/* A reward Twitch named: the reward's art and its name */}
                                                            <div className="relative flex-shrink-0">
                                                                {(notification.data as TwitchRewardNotificationData).thumbnail_url ? (
                                                                    <img
                                                                        src={(notification.data as TwitchRewardNotificationData).thumbnail_url}
                                                                        alt=""
                                                                        className="w-[34px] h-[34px] rounded-lg object-contain"
                                                                    />
                                                                ) : (
                                                                    <div className="sn-notif-glyph">
                                                                        <Award size={21} className="text-cyan-400" />
                                                                    </div>
                                                                )}
                                                            </div>
                                                            <div className="flex-1 min-w-0">
                                                                <div className="flex items-center gap-2">
                                                                    <Award size={14} className="text-cyan-400 flex-shrink-0" />
                                                                    <span className="text-white text-sm font-semibold truncate">
                                                                        {(notification.data as TwitchRewardNotificationData).kind === 'badge' ? 'Badge earned' : 'Drop reward'}
                                                                    </span>
                                                                </div>
                                                                <p className="text-white/50 text-sm truncate mt-0.5">
                                                                    {(notification.data as TwitchRewardNotificationData).body_spans.map((span, i) => (
                                                                        <span key={i} className={span.bold ? 'text-white/80 font-semibold' : undefined}>
                                                                            {span.text}
                                                                        </span>
                                                                    ))}
                                                                </p>
                                                            </div>
                                                        </>
                                                    ) : notification.type === 'membership_gift' ? (
                                                        <>
                                                            {/* Membership someone gave this member */}
                                                            <div className="relative flex-shrink-0">
                                                                <div className="sn-notif-glyph">
                                                                    <Sparkle size={21} weight="fill" className="text-amber-400" />
                                                                </div>
                                                            </div>
                                                            <div className="flex-1 min-w-0">
                                                                <div className="flex items-center gap-2">
                                                                    <span className="text-white text-sm font-semibold truncate">
                                                                        StreamNook membership
                                                                    </span>
                                                                </div>
                                                                <p className="text-white/50 text-sm line-clamp-2 mt-0.5">
                                                                    {(notification.data as MembershipGiftNotificationData).bodyPlain
                                                                        ?? ((notification.data as MembershipGiftNotificationData).permanent
                                                                            ? 'Somebody gave you a membership. It does not expire.'
                                                                            : 'Somebody gave you a membership. Your perks are active now.')}
                                                                </p>
                                                            </div>
                                                        </>
                                                    ) : notification.type === 'update' ? (
                                                        <>
                                                            {/* Update notification */}
                                                            <div className="relative flex-shrink-0">
                                                                <div className="sn-notif-glyph">
                                                                    <Download size={21} className="text-yellow-400" />
                                                                </div>
                                                            </div>
                                                            <div className="flex-1 min-w-0">
                                                                <div className="flex items-center gap-2">
                                                                    <Download size={14} className="text-yellow-400 flex-shrink-0" />
                                                                    <span className="text-white text-sm font-semibold truncate">
                                                                        Update Available
                                                                    </span>
                                                                </div>
                                                                <p className="text-white/50 text-sm truncate mt-0.5">
                                                                    v{(notification.data as UpdateNotificationData).current_version} → v{(notification.data as UpdateNotificationData).latest_version}
                                                                </p>
                                                            </div>
                                                        </>
                                                    ) : notification.type === 'drops' ? (
                                                        <>
                                                            {/* Drops notification */}
                                                            <div className="relative flex-shrink-0">
                                                                {(notification.data as DropsNotificationData).benefit_image_url ? (
                                                                    <img
                                                                        src={(notification.data as DropsNotificationData).benefit_image_url}
                                                                        alt=""
                                                                        className="w-[34px] h-[34px] rounded-lg object-cover"
                                                                    />
                                                                ) : (
                                                                    <div className="sn-notif-glyph">
                                                                        <Package size={21} className="text-green-400" />
                                                                    </div>
                                                                )}
                                                            </div>
                                                            <div className="flex-1 min-w-0">
                                                                <div className="flex items-center gap-2">
                                                                    {/* Only alongside the drop's own artwork. Without
                                                                        it the leading slot already carries this exact
                                                                        mark, and two of them on one row is what the
                                                                        channel-points row was doing. */}
                                                                    {!!(notification.data as DropsNotificationData).benefit_image_url && (
                                                                        <Package size={14} className="text-green-400 flex-shrink-0" />
                                                                    )}
                                                                    <span className="text-white text-sm font-semibold truncate">
                                                                        Drop Claimed
                                                                    </span>
                                                                </div>
                                                                <p className="text-white/50 text-sm truncate mt-0.5">
                                                                    {(notification.data as DropsNotificationData).game_name
                                                                        ? `${(notification.data as DropsNotificationData).drop_name} (${(notification.data as DropsNotificationData).game_name})`
                                                                        : (notification.data as DropsNotificationData).drop_name}
                                                                </p>
                                                            </div>
                                                        </>
                                                    ) : notification.type === 'channel_points' ? (
                                                        <>
                                                            {/* Channel Points notification: one channel shows its
                                                                face with its points icon attached; a mixed burst
                                                                keeps the plain mark. */}
                                                            {(notification.data as ChannelPointsNotificationData).avatar_url ? (
                                                                <PointsFace
                                                                    avatarUrl={(notification.data as ChannelPointsNotificationData).avatar_url as string}
                                                                    iconUrl={(notification.data as ChannelPointsNotificationData).icon_url}
                                                                />
                                                            ) : (
                                                                <div className="relative flex-shrink-0">
                                                                    <div className="sn-notif-glyph">
                                                                        <PointsGlyph size={18} />
                                                                    </div>
                                                                </div>
                                                            )}
                                                            <div className="flex-1 min-w-0">
                                                                {/* One points mark per row. The leading glyph already
                                                                    says what this is; a second copy beside the title was
                                                                    the same icon twice on one line. */}
                                                                <div className="flex items-center gap-2">
                                                                    <span className="text-white text-sm font-semibold truncate">
                                                                        {(notification.data as ChannelPointsNotificationData).points_name || 'Channel Points'} +{(notification.data as ChannelPointsNotificationData).points_earned.toLocaleString()}
                                                                    </span>
                                                                </div>
                                                                <p className="text-white/50 text-sm truncate mt-0.5">
                                                                    {(() => {
                                                                        const data = notification.data as ChannelPointsNotificationData;
                                                                        const channelName = data.channel_name;
                                                                        const totalPoints = data.total_points;
                                                                        // Check if channel_name looks like a reason summary (contains parentheses)
                                                                        const isReasonSummary = channelName && channelName.includes('(');
                                                                        if (isReasonSummary) {
                                                                            // Just show the reason summary without "for"
                                                                            return channelName;
                                                                        } else if (channelName && totalPoints) {
                                                                            // Real channel name with total - show "channelname: total points"
                                                                            return `${channelName}: ${totalPoints.toLocaleString()} ${data.points_name || 'points'}`;
                                                                        } else if (channelName) {
                                                                            // Real channel name without total - just show channel name
                                                                            return channelName;
                                                                        } else {
                                                                            // No channel name at all
                                                                            return 'Points earned';
                                                                        }
                                                                    })()}
                                                                </p>
                                                            </div>
                                                        </>
                                                    ) : notification.type === 'badge' ? (
                                                        <>
                                                            {/* Badge notification */}
                                                            <div className="relative flex-shrink-0">
                                                                {(notification.data as BadgeNotificationData).badge_image_url ? (
                                                                    <img
                                                                        src={(notification.data as BadgeNotificationData).badge_image_url}
                                                                        alt=""
                                                                        className="w-[34px] h-[34px] rounded-lg object-cover"
                                                                    />
                                                                ) : (
                                                                    <div className="sn-notif-glyph">
                                                                        <Award size={21} className="text-cyan-400" />
                                                                    </div>
                                                                )}
                                                                {/* Status indicator */}
                                                                {(notification.data as BadgeNotificationData).status === 'available' && (
                                                                    <div className="absolute -bottom-0.5 -right-0.5 w-3.5 h-3.5 bg-green-500 rounded-full border-2 border-black" />
                                                                )}
                                                                {(notification.data as BadgeNotificationData).status === 'coming_soon' && (
                                                                    <div className="absolute -bottom-0.5 -right-0.5 w-3.5 h-3.5 bg-blue-500 rounded-full border-2 border-black" />
                                                                )}
                                                            </div>
                                                            <div className="flex-1 min-w-0">
                                                                <div className="flex items-center gap-2">
                                                                    <Award size={14} className="text-cyan-400 flex-shrink-0" />
                                                                    <span className="text-white text-sm font-semibold truncate">
                                                                        {(notification.data as BadgeNotificationData).badge_name}
                                                                    </span>
                                                                </div>
                                                                <p className="text-white/50 text-sm truncate mt-0.5">
                                                                    {(() => {
                                                                        const data = notification.data as BadgeNotificationData;
                                                                        // Derived here, not read off the stored row: a badge
                                                                        // saved while upcoming would otherwise keep saying
                                                                        // "Coming soon" long after its window opened.
                                                                        const derived = windowStatusAt(data.window);
                                                                        const effective = derived === 'available' ? 'available'
                                                                            : derived === 'coming-soon' ? 'coming_soon'
                                                                            : data.status;
                                                                        const statusText = derived === 'expired' ? 'Ended' :
                                                                            effective === 'new' ? 'New badge' :
                                                                            effective === 'available' ? 'Now available' : 'Coming soon';
                                                                        const when = formatBadgeDateInfo(data.date_info);
                                                                        return when ? `${statusText} • ${when}` : statusText;
                                                                    })()}
                                                                </p>
                                                            </div>
                                                        </>
                                                    ) : notification.type === 'system' ? (
                                                        <>
                                                            {/* Action feedback mirrored from a toast */}
                                                            {(() => {
                                                                const d = notification.data as SystemNotificationData;
                                                                // Source beats level: what a plugin has to say is more
                                                                // useful than the fact that it went fine.
                                                                const Icon = d.source === 'plugin' ? PuzzlePiece
                                                                    : d.level === 'success' ? CheckCircle2
                                                                    : d.level === 'error' ? XCircle
                                                                    : d.level === 'warning' ? AlertTriangle
                                                                    : Info;
                                                                const color = d.source === 'plugin' ? 'text-accent'
                                                                    : d.level === 'success' ? 'text-green-400'
                                                                    : d.level === 'error' ? 'text-red-400'
                                                                    : d.level === 'warning' ? 'text-yellow-400'
                                                                    : 'text-accent';
                                                                return (
                                                                    <>
                                                                        <div className="relative flex-shrink-0">
                                                                            {d.avatar_url ? (
                                                                                <>
                                                                                    <SevenTvAvatar
                                                                                        cosmeticsId={d.avatar_seventv_id}
                                                                                        px={34}
                                                                                        motion="hover"
                                                                                        src={d.avatar_url}
                                                                                        alt=""
                                                                                        className="w-[34px] h-[34px] rounded-full object-cover"
                                                                                    />
                                                                                    {/* The glyph survives as a small seal on the
                                                                                        face, so the row still says at a glance
                                                                                        whether it went well without giving the
                                                                                        whole circle over to a tick. */}
                                                                                    <span className="sn-notif-seal">
                                                                                        <Icon size={13} className={color} />
                                                                                    </span>
                                                                                </>
                                                                            ) : (
                                                                                <div className="sn-notif-glyph">
                                                                                    <Icon size={21} className={color} />
                                                                                </div>
                                                                            )}
                                                                        </div>
                                                                        <div className="flex-1 min-w-0">
                                                                            <p className="text-white/90 text-sm line-clamp-2">
                                                                                {d.message}
                                                                            </p>
                                                                        </div>
                                                                    </>
                                                                );
                                                            })()}
                                                        </>
                                                    ) : null}

                                                    {/* Time, swapped for the row's actions on hover. One
                                                        fixed slot for every row, wide enough for the most
                                                        actions a row has, so the times share one edge and
                                                        text wraps at the same place whatever the row offers. */}
                                                    <div className="relative flex w-[62px] flex-shrink-0 justify-end self-start pt-[3px]">
                                                        <span className="text-[10.5px] text-white/30 transition-opacity group-hover:opacity-0 group-focus-within:opacity-0">
                                                            {formatTimeAgo(notification.timestamp)}
                                                        </span>
                                                        <div className="absolute right-0 top-0 flex items-center gap-1 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">
                                                        {/* Mark as read button - only show for unread notifications */}
                                                        {!notification.read && (
                                                            <Tooltip content="Mark as read" side="top">
                                                                <button
                                                                    type="button"
                                                                    aria-label="Mark as read"
                                                                    onClick={(e) => notification.type === 'mention'
                                                                        ? markMentionRead(notification.data, e)
                                                                        : markAsRead(notification.id, e)}
                                                                    className="text-white/30hover:text-green-400 transition-all p-0.5"
                                                                    data-keeps-player-fullscreen
                                                                >
                                                                    <Check size={14} />
                                                                </button>
                                                            </Tooltip>
                                                        )}
                                                        {notification.type === 'mention' && (() => {
                                                            const muted = mutedRoomSet.has(notification.data.room.toLowerCase());
                                                            const label = muted ? 'Unmute this chat' : 'Mute this chat';
                                                            return (
                                                                <Tooltip content={label} side="top">
                                                                    <button
                                                                        type="button"
                                                                        aria-label={label}
                                                                        onClick={(e) => toggleMentionMute(notification.data, e)}
                                                                        className="text-white/30hover:text-white transition-all p-0.5"
                                                                        data-keeps-player-fullscreen
                                                                    >
                                                                        {muted ? <Bell size={14} /> : <BellOff size={14} />}
                                                                    </button>
                                                                </Tooltip>
                                                            );
                                                        })()}
                                                        <Tooltip content={notification.type === 'mention' ? 'Remove' : 'Remove notification'} side="top">
                                                            <button
                                                                type="button"
                                                                aria-label={notification.type === 'mention' ? 'Remove' : 'Remove notification'}
                                                                onClick={(e) => notification.type === 'mention'
                                                                    ? removeMention(notification.data, e)
                                                                    : clearNotification(notification.id, e)}
                                                                className="text-white/30hover:text-white transition-all p-0.5"
                                                                data-keeps-player-fullscreen
                                                            >
                                                                <X size={14} />
                                                            </button>
                                                        </Tooltip>
                                                        </div>
                                                    </div>
                                                </motion.div>
                                            </Fragment>
                                            );
                                            })}
                                        </div>
                                    )}
                                </div>
                            </motion.div>
                        )}
                    </AnimatePresence>
                </motion.div>
            </div>
        </>
    );
};

export default DynamicIsland;
