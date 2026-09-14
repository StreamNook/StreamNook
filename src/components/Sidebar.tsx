import { useEffect, useState, useRef, useCallback, useMemo, memo } from 'react';
import { useShallow } from 'zustand/react/shallow';
import { useAppStore, ensureHomeSnapshotSync } from '../stores/AppStore';
import { ChevronLeft, ChevronRight, Users, Sparkles, Radio, Heart, Gift, Flame, Star } from 'lucide-react';
import type { TwitchStream } from '../types';
import { invoke } from '@tauri-apps/api/core';
import { getSidebarSettings, type SidebarMode } from './settings/InterfaceSettings';
import { WATCHABLE_PROVIDERS, providerLabel, type ProviderId } from '../types/providers';

import { useContextMenuStore } from '../stores/contextMenuStore';
import { usemultiNookStore } from '../stores/multiNookStore';
import { Tooltip } from './ui/Tooltip';
import StreamHoverCard, { STREAM_HOVER_CARD_CLASS } from './StreamHoverCard';
import { ProviderLogo } from './ProviderLogo';
import { useFollowsStore } from '../stores/followsStore';
import { useFavoritesStore } from '../stores/favoritesStore';
import { favoriteIdOf, favoriteMetaOf, dedupeByFavoriteId } from '../utils/favorites';
import { streamProvider, streamKey } from '../utils/streamProvider';
import { useStreamAvatars } from '../hooks/useStreamAvatars';

import { Logger } from '../utils/logger';
import { formatViewerCount } from '../utils/streamStats';
// Width constants
const COMPACT_WIDTH = 56;
const DEFAULT_EXPANDED_WIDTH = 280;
const MIN_EXPANDED_WIDTH = 200;
const MAX_EXPANDED_WIDTH = 450;
const HIDDEN_TRIGGER_ZONE = 16; // pixels from left edge to trigger sidebar
const SIDEBAR_CLOSE_DELAY = 150; // milliseconds delay before closing in hidden mode
const SIDEBAR_EXPAND_MS = 200; // width-animation duration for compact / expand-on-hover
const SIDEBAR_BLUR_SETTLE_DELAY = SIDEBAR_EXPAND_MS + 40; // fade the glass in just after the expand settles
const SIDEBAR_WIDTH_STORAGE_KEY = 'sidebar-expanded-width';

// Get persisted sidebar width from localStorage
const getPersistedWidth = (): number => {
    try {
        const saved = localStorage.getItem(SIDEBAR_WIDTH_STORAGE_KEY);
        if (saved) {
            const width = parseInt(saved, 10);
            if (width >= MIN_EXPANDED_WIDTH && width <= MAX_EXPANDED_WIDTH) {
                return width;
            }
        }
    } catch (e) {
        Logger.error('[Sidebar] Failed to read persisted width:', e);
    }
    return DEFAULT_EXPANDED_WIDTH;
};

// Save sidebar width to localStorage
const persistWidth = (width: number): void => {
    try {
        localStorage.setItem(SIDEBAR_WIDTH_STORAGE_KEY, width.toString());
    } catch (e) {
        Logger.error('[Sidebar] Failed to persist width:', e);
    }
};

// Pure helper: format a viewer count like 12300 -> "12.3K".

type HypeTrainStatus = { level: number; isGolden: boolean };

interface SectionHeaderProps {
    icon: any;
    label: string;
    count: number;
    showExpanded: boolean;
    /** Extra classes for the icon, e.g. the favorites heart's pink fill. */
    iconClassName?: string;
    /** Fully-rendered icon override, for marks that need SVG attrs a
     *  className can't carry (the glass-gradient heart). */
    iconNode?: React.ReactNode;
}

// Hoisted to module scope (and memoized) so it keeps a stable component
// identity across Sidebar re-renders. Defining it inside Sidebar made React
// treat it as a brand-new component type on every render, which unmounted and
// remounted the whole list on each background refresh — the visible "glitch".
const SectionHeader = memo(({ icon: Icon, label, count, showExpanded, iconClassName = '', iconNode }: SectionHeaderProps) => (
    <div className={`
        flex items-center gap-2 px-2 py-2 text-textSecondary
        ${showExpanded ? 'justify-start' : 'justify-center'}
    `}>
        {iconNode ?? <Icon size={16} className={`flex-shrink-0 ${iconClassName}`} />}
        {showExpanded && (
            <>
                <span className="text-xs font-semibold uppercase tracking-wider">{label}</span>
                <span className="text-xs text-textMuted">({count})</span>
            </>
        )}
    </div>
));

interface StreamItemProps {
    stream: TwitchStream;
    showFavorite?: boolean;
    showExpanded: boolean;
    isCurrentStream: boolean;
    isFavorite: boolean;
    /** Subscribed to this channel on its platform (imported with the follows). */
    isSubscribed: boolean;
    /** The list is showing more than one platform, so rows need marking. */
    showPlatformBadge: boolean;
    hasDrops: boolean;
    hypeTrainStatus: HypeTrainStatus | undefined;
    watchStreak: number;
    isHeartAnimating: boolean;
    profileImage: string;
    onStreamClick: (e: React.MouseEvent, stream: TwitchStream) => void;
    onFavoriteClick: (e: React.MouseEvent, stream: TwitchStream) => void;
}

// Hoisted + memoized for the same reason as SectionHeader: a stable identity
// means a background stream refresh reconciles each row in place (text/badges
// update) instead of tearing the avatar <img> down and replaying its fade-in.
// All per-row data is passed in as props so memo can skip rows whose data is
// unchanged when an unrelated piece of state (e.g. hover) updates the parent.
const StreamItem = memo(({
    stream,
    showFavorite = false,
    showExpanded,
    isCurrentStream,
    isFavorite,
    isSubscribed,
    showPlatformBadge,
    hasDrops,
    hypeTrainStatus,
    watchStreak,
    isHeartAnimating,
    profileImage,
    onStreamClick,
    onFavoriteClick,
}: StreamItemProps) => {
    return (
        <Tooltip
            content={<StreamHoverCard stream={stream} hasDrops={hasDrops} />}
            containerClassName={STREAM_HOVER_CARD_CLASS}
            delay={300}
            side="right"
        >
            <div
                aria-label={`${stream.user_name} - ${stream.title}`}
                className={`group
                    flex items-center px-2 py-1.5 cursor-pointer rounded transition-all duration-200
                    ${isCurrentStream
                        ? 'border-l-2 border-accent hover:bg-surface-hover'
                        : 'hover:bg-surface-hover border-l-2 border-transparent'
                    }
                    ${showExpanded ? 'gap-2 justify-start' : 'gap-0 justify-center'}
                `}
                onClick={(e) => onStreamClick(e, stream)}
                onContextMenu={(e) => useContextMenuStore.getState().openMenu(e, stream)}
            >
            {/* Avatar with live indicator */}
            <div className="relative flex-shrink-0 transition-all duration-200">
                <img
                    src={profileImage}
                    alt={stream.user_name}
                    className={`rounded-full object-cover transition-all duration-200 ${showExpanded ? 'w-8 h-8' : 'w-9 h-9'}`}
                    onError={(e) => {
                        (e.target as HTMLImageElement).src = 'https://static-cdn.jtvnw.net/user-default-pictures-uv/75305d54-c7cc-40d1-bb9c-91c46bf27829-profile_image-70x70.png';
                    }}
                />
                {/* Live presence dot: static at rest (the sidebar only ever lists
                    live channels, so a per-row pulse is redundant and, stacked
                    across an expanded list, needless idle animation). Reuses the
                    stream-card `pulse-dot` keyframes (transform-scale, GPU-cheap)
                    and only animates on the hovered row via group-hover. */}
                <div className="absolute -bottom-0.5 -right-0.5 w-3 h-3 rounded-full bg-live border-2 border-background group-hover:animate-[pulse-dot_2s_ease-in-out_infinite]" />
                {/* Which platform this channel is on — shown ONLY while the list
                    is mixed, and then on EVERY row including Twitch, since with
                    platforms mixed an unmarked row would be a guess rather than a
                    default. Scoped to one platform it disappears entirely. */}
                {/* The bare mark floats on the avatar — no disc behind it. A
                    background plate reads as a UI chip stuck to the picture; the
                    logo alone reads as what it is. A soft shadow keeps it legible
                    over a light avatar without drawing a container. */}
                {showPlatformBadge && (
                    <div className="absolute -top-1 -left-1 flex items-center justify-center drop-shadow-[0_1px_2px_rgba(0,0,0,0.85)]">
                        <ProviderLogo provider={streamProvider(stream)} size={12} />
                    </div>
                )}
                {/* Subscribed on that platform (imported with the follow list). */}
                {isSubscribed && (
                    <Tooltip content="Subscribed" delay={100} side="right">
                        <div className="absolute -bottom-0.5 -left-0.5 w-3 h-3 rounded-full bg-accent border-2 border-background flex items-center justify-center">
                            <Star size={6} className="text-white" fill="currentColor" />
                        </div>
                    </Tooltip>
                )}
                {/* Drops indicator on avatar - only show in compact mode */}
                {hasDrops && !showExpanded && (
                    <div className="absolute -top-1 -right-1 w-4 h-4 rounded-full bg-accent flex items-center justify-center border border-background">
                        <Gift size={10} className="text-white" />
                    </div>
                )}
                {/* Hype Train indicator on avatar - only show in compact mode when no drops */}
                {hypeTrainStatus && !showExpanded && !hasDrops && (
                    <Tooltip content={`Hype Train LVL ${hypeTrainStatus.level}`} delay={100} side="right">
                        <div className={`absolute -top-1 -right-1 w-4 h-4 rounded-full flex items-center justify-center border border-background ${hypeTrainStatus.isGolden ? 'bg-yellow-500' : 'bg-purple-600'}`}>
                            <svg className="w-2.5 h-2.5 text-white" viewBox="0 0 15 13" fill="none">
                                <path fillRule="evenodd" clipRule="evenodd" d="M4.10001 0.549988H2.40001V4.79999H0.700012V10.75H1.55001C1.55001 11.6889 2.31113 12.45 3.25001 12.45C4.1889 12.45 4.95001 11.6889 4.95001 10.75H5.80001C5.80001 11.6889 6.56113 12.45 7.50001 12.45C8.4389 12.45 9.20001 11.6889 9.20001 10.75H10.05C10.05 11.6889 10.8111 12.45 11.75 12.45C12.6889 12.45 13.45 11.6889 13.45 10.75H14.3V0.549988H6.65001V2.24999H7.50001V4.79999H4.10001V0.549988ZM12.6 9.04999V6.49999H2.40001V9.04999H12.6ZM9.20001 4.79999H12.6V2.24999H9.20001V4.79999Z" fill="currentColor" />
                            </svg>
                        </div>
                    </Tooltip>
                )}
            </div>

            {/* Stream info - only show when expanded */}
            {showExpanded && (
                <div className="flex-1 min-w-0 overflow-hidden animate-fade-in">
                    <div className="flex items-center gap-1">
                        <span className="text-textPrimary text-sm font-medium truncate">
                            {stream.user_name}
                        </span>
                        {stream.broadcaster_type === 'partner' && (
                            <svg className="w-3 h-3 flex-shrink-0" viewBox="0 0 16 16" fill="#9146FF">
                                <path fillRule="evenodd" d="M12.5 3.5 8 2 3.5 3.5 2 8l1.5 4.5L8 14l4.5-1.5L14 8l-1.5-4.5ZM7 11l4.5-4.5L10 5 7 8 5.5 6.5 4 8l3 3Z" clipRule="evenodd" />
                            </svg>
                        )}
                        {watchStreak > 0 && (
                            <Tooltip content={`${watchStreak} Watch Streak`} delay={200} side="top">
                                <div className="flex items-center gap-[2px] ml-0.5 text-orange-400 opacity-90 transition-opacity hover:opacity-100 cursor-default">
                                    <span className="text-[10px] font-bold leading-none translate-y-[0.5px]">{watchStreak}</span>
                                    <Flame size={12} strokeWidth={2.5} className="text-orange-500 fill-orange-500/20" />
                                </div>
                            </Tooltip>
                        )}
                    </div>
                    <div className="flex items-center gap-1 text-xs text-textMuted truncate">
                        <span className="truncate">{stream.game_name || 'Just Chatting'}</span>
                        {/* Drops indicator - show next to game name */}
                        {hasDrops && (
                            <Tooltip content="Drops enabled" delay={100} side="top">
                                <span>
                                    <Gift size={10} className="text-accent flex-shrink-0" />
                                </span>
                            </Tooltip>
                        )}
                        {/* Hype Train indicator - show next to game name */}
                        {hypeTrainStatus && (
                            <Tooltip content={`Hype Train LVL ${hypeTrainStatus.level}`} delay={100} side="top">
                                <span className={`flex items-center gap-0.5 ${hypeTrainStatus.isGolden ? 'text-yellow-400' : 'text-purple-400'} flex-shrink-0`}>
                                    <svg className="w-2.5 h-2.5" viewBox="0 0 15 13" fill="none">
                                        <path fillRule="evenodd" clipRule="evenodd" d="M4.10001 0.549988H2.40001V4.79999H0.700012V10.75H1.55001C1.55001 11.6889 2.31113 12.45 3.25001 12.45C4.1889 12.45 4.95001 11.6889 4.95001 10.75H5.80001C5.80001 11.6889 6.56113 12.45 7.50001 12.45C8.4389 12.45 9.20001 11.6889 9.20001 10.75H10.05C10.05 11.6889 10.8111 12.45 11.75 12.45C12.6889 12.45 13.45 11.6889 13.45 10.75H14.3V0.549988H6.65001V2.24999H7.50001V4.79999H4.10001V0.549988ZM12.6 9.04999V6.49999H2.40001V9.04999H12.6ZM9.20001 4.79999H12.6V2.24999H9.20001V4.79999Z" fill="currentColor" />
                                    </svg>
                                </span>
                            </Tooltip>
                        )}
                    </div>
                </div>
            )}

            {/* Viewer count and favorite button */}
            {showExpanded && (
                <div className="flex items-center gap-1 flex-shrink-0 animate-fade-in">
                    <div className="flex items-center gap-1 text-xs text-textSecondary">
                        <Radio size={10} className="text-live" />
                        <span>{formatViewerCount(stream.viewer_count)}</span>
                    </div>
                    {showFavorite && (
                        <Tooltip content={isFavorite ? 'Remove from favorites' : 'Add to favorites'} delay={200} side="top">
                            <button
                                onClick={(e) => onFavoriteClick(e, stream)}
                                className={`p-1 flex items-center justify-center bg-transparent transition-transform duration-300 hover:scale-110 active:scale-95`}
                            >
                                <Heart
                                    size={14}
                                    fill={isFavorite ? 'url(#glass-heart-fill)' : 'none'}
                                    stroke={isFavorite ? 'url(#glass-heart-stroke)' : 'currentColor'}
                                    strokeWidth={isFavorite ? 1.5 : 2}
                                    className={`transition-all duration-300 ${isFavorite ? 'drop-shadow-[0_4px_8px_color-mix(in_srgb,var(--color-highlight-pink)_50%,transparent)]' : 'text-textMuted hover:text-white opacity-0 group-hover:opacity-100'} ${isHeartAnimating ? 'animate-heart-break' : ''}`}
                                />
                            </button>
                        </Tooltip>
                    )}

                </div>
            )}
        </div>
    </Tooltip>
    );
});

const Sidebar = ({ side = 'left' }: { side?: 'left' | 'right' }) => {
    // Mirror the sidebar to the right edge (used when chat is docked left with
    // reveal-on-hover, so the left edge belongs to the chat and the two don't fight
    // over the same hover zone).
    const onRight = side === 'right';
    // Actions without a subscription (stable for the store's lifetime); state
    // through a shallow-compared selector, so an unrelated store tick no longer
    // re-renders the whole sidebar.
    const {
        loadFollowedStreams,
        loadRecommendedStreams,
        loadMoreRecommendedStreams,
        isFavoriteStreamer,
    } = useAppStore.getState();
    const {
        followedStreams,
        recommendedStreams,
        hasMoreRecommended,
        isLoadingMore,
        currentStream,
        isAuthenticated,
        // Hype Train status for stream badges
        activeHypeTrainChannels,
        watchStreaks,
    } = useAppStore(
        useShallow((s) => ({
            followedStreams: s.followedStreams,
            recommendedStreams: s.recommendedStreams,
            hasMoreRecommended: s.hasMoreRecommended,
            isLoadingMore: s.isLoadingMore,
            currentStream: s.currentStream,
            isAuthenticated: s.isAuthenticated,
            activeHypeTrainChannels: s.activeHypeTrainChannels,
            watchStreaks: s.watchStreaks,
            // Not destructured, and still load-bearing: `isFavoriteStreamer` is
            // called during render (the Favorites section, the heart on each
            // row) and reads settings.favorite_streamers, which is not itself
            // reactive. Tracking the slice here is what re-renders on a toggle.
            // Only the membership list: the identity sidecar
            // (settings.favorite_channels) feeds Home's offline roster, and the
            // sidebar lists live channels only.
            favoriteStreamers: s.settings.favorite_streamers,
        })),
    );


    // Sidebar mode from settings
    const [sidebarMode, setSidebarMode] = useState<SidebarMode>(() => {
        const settings = getSidebarSettings();
        return settings.mode;
    });
    const [expandOnHover, setExpandOnHover] = useState(() => {
        const settings = getSidebarSettings();
        return settings.expandOnHover;
    });
    // Which platform the app is scoped to. Declared up here with the other
    // top-level state because the scroll effect below reads it.
    const activePlatform = useAppStore((s) => s.activePlatform);
    const [showRecommended, setShowRecommended] = useState(() => {
        const settings = getSidebarSettings();
        return settings.showRecommended;
    });

    // Hover and manual expand states
    const [isHovered, setIsHovered] = useState(false);
    const [isEdgeHovered, setIsEdgeHovered] = useState(false);
    const [isManuallyExpanded, setIsManuallyExpanded] = useState(false);

    // Resizable width state
    const [expandedWidth, setExpandedWidth] = useState<number>(getPersistedWidth);
    const [isResizing, setIsResizing] = useState(false);

    // Whether the frosted-glass blur has faded in. It is held back until the
    // panel has finished expanding so the blur is never re-rasterized mid-resize.
    const [blurReady, setBlurReady] = useState(false);

    const sidebarRef = useRef<HTMLDivElement>(null);
    const scrollContainerRef = useRef<HTMLDivElement>(null);
    const edgeTriggerRef = useRef<HTMLDivElement>(null);
    const closeTimeoutRef = useRef<NodeJS.Timeout | null>(null);
    const [animatingHearts, setAnimatingHearts] = useState<Set<string>>(new Set());

    // Cache for profile images fetched from Twitch Helix API

    // Games with a drop campaign the account is actively in, from the Rust
    // Home snapshot (hourly, and on Home mount when stale).
    const dropsActiveGameNames = useAppStore((s) => s.dropsActiveGameNames);
    const dropsGameNames = useMemo(() => new Set(dropsActiveGameNames), [dropsActiveGameNames]);

    // Hype-train badges, the followed list, recommended and the drops indicator
    // arrive from the Rust Home snapshot (polled in Rust, emitted only on
    // change); the JS pollers that lived here are gone. Registering the sync
    // here covers windows that boot straight into a stream, where Home never
    // mounts.
    useEffect(() => {
        void ensureHomeSnapshotSync();
    }, []);

    // Listen for settings changes from InterfaceSettings
    useEffect(() => {
        const handleSettingsChange = (event: CustomEvent<{ mode: SidebarMode; expandOnHover: boolean; showRecommended: boolean }>) => {
            setSidebarMode(event.detail.mode);
            setExpandOnHover(event.detail.expandOnHover);
            setShowRecommended(event.detail.showRecommended);
        };

        window.addEventListener('sidebar-settings-changed', handleSettingsChange as EventListener);
        return () => window.removeEventListener('sidebar-settings-changed', handleSettingsChange as EventListener);
    }, []);

    // Cleanup close timeout on unmount
    useEffect(() => {
        return () => {
            if (closeTimeoutRef.current) {
                clearTimeout(closeTimeoutRef.current);
            }
        };
    }, []);

    // Close sidebar when mouse leaves the document/window entirely (for hidden mode)
    useEffect(() => {
        if (sidebarMode !== 'hidden') return;

        const handleDocumentMouseLeave = (e: MouseEvent) => {
            // Don't close while resizing
            if (isResizing) return;
            // Check if mouse is actually leaving the document (not just moving to another element)
            if (e.relatedTarget === null) {
                setIsHovered(false);
                setIsEdgeHovered(false);
            }
        };

        const handleWindowBlur = () => {
            // Don't close while resizing
            if (isResizing) return;
            // Close sidebar when app loses focus
            setIsHovered(false);
            setIsEdgeHovered(false);
        };

        // Track mouse position to detect if cursor is outside the viewport
        const handleMouseMove = (e: MouseEvent) => {
            // Don't close while resizing
            if (isResizing) return;
            // If mouse moves and we're in hidden mode with sidebar visible,
            // check if mouse is still within sidebar bounds
            if ((isHovered || isEdgeHovered) && sidebarRef.current) {
                const rect = sidebarRef.current.getBoundingClientRect();
                const isInsideSidebar =
                    e.clientX >= 0 &&
                    e.clientX <= rect.right &&
                    e.clientY >= rect.top &&
                    e.clientY <= rect.bottom;

                // Also check edge trigger zone
                const isInEdgeZone = e.clientX >= 0 && e.clientX <= HIDDEN_TRIGGER_ZONE;

                if (!isInsideSidebar && !isInEdgeZone) {
                    setIsHovered(false);
                    setIsEdgeHovered(false);
                }
            }
        };

        document.addEventListener('mouseleave', handleDocumentMouseLeave);
        window.addEventListener('blur', handleWindowBlur);
        document.addEventListener('mousemove', handleMouseMove);

        return () => {
            document.removeEventListener('mouseleave', handleDocumentMouseLeave);
            window.removeEventListener('blur', handleWindowBlur);
            document.removeEventListener('mousemove', handleMouseMove);
        };
    }, [sidebarMode, isHovered, isEdgeHovered, isResizing]);

    // Load streams on mount and when auth changes
    useEffect(() => {
        if (isAuthenticated) {
            loadFollowedStreams();
        }
        loadRecommendedStreams();
    }, [isAuthenticated, loadFollowedStreams, loadRecommendedStreams]);

    // Track previous "sidebar visible" state to detect rising edge (opening)
    const prevSidebarVisibleRef = useRef(false);

    // Background Sync & Layout Shift Prevention
    // Instead of fetching on OPEN (which causes a 500ms delay followed by an instant layout jump
    // as stream arrays reshuffle), we fetch on CLOSE and via a slow background interval.
    useEffect(() => {
        const isSidebarVisible = isHovered || isEdgeHovered || isManuallyExpanded;
        const wasVisible = prevSidebarVisibleRef.current;
        prevSidebarVisibleRef.current = isSidebarVisible;

        // In expanded mode the sidebar is permanently open, so a hover-out is not
        // a "close" — refreshing here just reshuffles the list under the user's
        // cursor. The periodic background sync keeps expanded mode fresh instead.
        if (sidebarMode === 'expanded') return;

        // Refresh on falling edge (sidebar just closed)
        if (!isSidebarVisible && wasVisible) {
            Logger.debug('[Sidebar] Refreshing streams on sidebar close (Layout Shift Prevention)');
            if (isAuthenticated) {
                loadFollowedStreams();
            }
            loadRecommendedStreams();
        }
    }, [isHovered, isEdgeHovered, isManuallyExpanded, isAuthenticated, loadFollowedStreams, loadRecommendedStreams, sidebarMode]);


    // Infinite scroll for recommended streams
    useEffect(() => {
        const scrollContainer = scrollContainerRef.current;
        if (!scrollContainer) return;

        const handleScroll = () => {
            const { scrollTop, scrollHeight, clientHeight } = scrollContainer;
            // Paging applies to Twitch's recommendations only. Scoped to another
            // platform this would fetch Twitch rows that the list will never
            // show — its directory arrives as one ranked page.
            const paginates = activePlatform === 'all' || activePlatform === 'twitch';
            if (paginates && showRecommended && scrollHeight - scrollTop - clientHeight < 100 && hasMoreRecommended && !isLoadingMore) {
                loadMoreRecommendedStreams();
            }
        };

        scrollContainer.addEventListener('scroll', handleScroll);
        return () => scrollContainer.removeEventListener('scroll', handleScroll);
    }, [showRecommended, hasMoreRecommended, isLoadingMore, loadMoreRecommendedStreams, activePlatform]);


    // Handle resize drag
    useEffect(() => {
        if (!isResizing) return;

        const handleMouseMove = (e: MouseEvent) => {
            const newWidth = Math.min(MAX_EXPANDED_WIDTH, Math.max(MIN_EXPANDED_WIDTH, e.clientX));
            setExpandedWidth(newWidth);
        };

        const handleMouseUp = () => {
            setIsResizing(false);
            // Persist the width when done resizing
            persistWidth(expandedWidth);
        };

        document.addEventListener('mousemove', handleMouseMove);
        document.addEventListener('mouseup', handleMouseUp);

        // Prevent text selection while resizing
        document.body.style.userSelect = 'none';
        document.body.style.cursor = 'ew-resize';

        return () => {
            document.removeEventListener('mousemove', handleMouseMove);
            document.removeEventListener('mouseup', handleMouseUp);
            document.body.style.userSelect = '';
            document.body.style.cursor = '';
        };
    }, [isResizing, expandedWidth]);

    // Calculate visibility and width based on mode
    const calculateSidebarState = useCallback(() => {
        switch (sidebarMode) {
            case 'expanded':
                // Always fully expanded
                return { visible: true, width: expandedWidth, showExpanded: true, isOverlay: false };

            case 'compact': {
                // Show compact, expand on hover if enabled, or if manually expanded
                const shouldExpand = isManuallyExpanded || (expandOnHover && isHovered);
                // When expand-on-hover is enabled, always use overlay mode (whether hovering or not)
                // This prevents layout jumps during expand/contract animations
                const isCompactOverlay = expandOnHover && !isManuallyExpanded;
                return {
                    visible: true,
                    width: shouldExpand ? expandedWidth : COMPACT_WIDTH,
                    showExpanded: shouldExpand,
                    isOverlay: isCompactOverlay
                };
            }

            case 'hidden': {
                // Hidden until edge hover, then fully expanded
                // Keep visible while resizing to allow drag to complete
                const isVisible = isEdgeHovered || isHovered || isResizing;
                return {
                    visible: isVisible,
                    width: isVisible ? expandedWidth : 0,
                    showExpanded: isVisible,
                    isOverlay: true
                };
            }

            case 'disabled':
                // Completely disabled - never show
                return { visible: false, width: 0, showExpanded: false, isOverlay: false };

            default:
                return { visible: true, width: COMPACT_WIDTH, showExpanded: false, isOverlay: false };
        }
    }, [sidebarMode, expandOnHover, isHovered, isEdgeHovered, isManuallyExpanded, expandedWidth, isResizing]);

    // Start resize handler
    const handleResizeStart = useCallback((e: React.MouseEvent) => {
        e.preventDefault();
        setIsResizing(true);
    }, []);

    // Avatars for the rows being rendered, across platforms: Twitch needs a Helix
    // users lookup, and YouTube CATEGORY rows need a per-channel resolve (search
    // and the subscriptions feed already ship theirs on the row).
    // Declared here rather than beside its fetch below so the avatar hook can see
    // it: the platform directory is the surface whose rows lack an avatar.
    const [platformTopLive, setPlatformTopLive] = useState<TwitchStream[]>([]);
    const providerRows = useMemo(
        () => [...followedStreams, ...recommendedStreams, ...platformTopLive],
        [followedStreams, recommendedStreams, platformTopLive],
    );
    const rowAvatars = useStreamAvatars(providerRows);

    // Get profile image - must be defined before early return to maintain hook order
    const getProfileImage = useCallback((stream: TwitchStream): string => {
        if (stream.profile_image_url) {
            return stream.profile_image_url;
        }
        const resolved = rowAvatars[streamKey(stream)];
        if (resolved) {
            return resolved;
        }
        if (stream.thumbnail_url) {
            return stream.thumbnail_url.replace('{width}', '150').replace('{height}', '150');
        }
        return `https://static-cdn.jtvnw.net/user-default-pictures-uv/75305d54-c7cc-40d1-bb9c-91c46bf27829-profile_image-70x70.png`;
    }, [rowAvatars]);

    // Stable across renders (defined before the early return to keep hook order
    // constant) so memoized StreamItem rows aren't invalidated by a new callback
    // identity each render. Reactive store values are read via getState() at call
    // time rather than captured in deps.
    const handleStreamClick = useCallback(async (e: React.MouseEvent, stream: TwitchStream) => {
        // Ctrl/Cmd+click adds the stream to multinook instead of switching to it.
        // The flying-card animation originates from the click point so it visually
        // matches the right-click context-menu "Add to MultiNook" action.
        if (e.ctrlKey || e.metaKey) {
            e.preventDefault();
            // No refusal here: addSlot owns the grid gate and reports its own
            // reason, so a second copy of the rule would only drift from it.
            const provider = streamProvider(stream);
            usemultiNookStore.getState().triggerAddAnimation(e.clientX, e.clientY, stream.user_login, provider);
            usemultiNookStore.getState().addSlot(stream.user_login, provider);
            return;
        }
        // Exit home/PIP mode when clicking on a new stream from sidebar
        // This ensures the user goes directly to the stream view
        const { isHomeActive, toggleHome, startStream } = useAppStore.getState();
        if (isHomeActive) {
            toggleHome();
        }

        // Leave MultiNook too. `App.tsx` gates the entire player on
        // `isMultiNookActive`, so the grid renders OVER the single-stream view:
        // starting a stream without leaving it ran the resolve for real, showed
        // its loading indicator for as long as that took, and then left the
        // viewer in the grid with nothing visibly changed.
        //
        // AWAITED, and the Home flag cleared afterwards, because the exit path
        // sets `isHomeActive` whenever no single stream is playing, which is
        // exactly the case here. Firing `startStream` before that resolves lands
        // the viewer on Home instead of the channel they clicked.
        const multiNook = usemultiNookStore.getState();
        if (multiNook.isMultiNookActive) {
            await multiNook.toggleMultiNook();
            useAppStore.setState({ isHomeActive: false });
        }

        // The row carries its platform, so startStream routes correctly.
        startStream(stream.user_login, stream);
    }, []);

    // Pending heart-break timers, keyed by favourite id. Un-favouriting is
    // deferred so the break animation can play; without cancelling, a
    // re-favourite inside that second let the old timer toggle it back off.
    const heartBreakTimers = useRef(new Map<string, ReturnType<typeof setTimeout>>());
    useEffect(() => {
        const timers = heartBreakTimers.current;
        return () => {
            for (const t of timers.values()) clearTimeout(t);
            timers.clear();
        };
    }, []);

    const handleFavoriteClick = useCallback((e: React.MouseEvent, stream: TwitchStream) => {
        e.stopPropagation();

        // `favoriteIdOf`, never `stream.user_id`: Kick and Twitch both use
        // numeric ids, so a Kick row could read as a Twitch favourite (a filled
        // heart on the wrong channel) and write a colliding key.
        const id = favoriteIdOf(stream);
        if (!id) return;

        const { isFavoriteStreamer, toggleFavoriteStreamer } = useAppStore.getState();

        const pending = heartBreakTimers.current.get(id);
        if (pending) {
            clearTimeout(pending);
            heartBreakTimers.current.delete(id);
            setAnimatingHearts(prev => {
                const next = new Set(prev);
                next.delete(id);
                return next;
            });
        }

        if (isFavoriteStreamer(id)) {
            setAnimatingHearts(prev => new Set(prev).add(id));
            const timer = setTimeout(() => {
                heartBreakTimers.current.delete(id);
                setAnimatingHearts(prev => {
                    const next = new Set(prev);
                    next.delete(id);
                    return next;
                });
                void toggleFavoriteStreamer(id);
            }, 1000);
            heartBreakTimers.current.set(id, timer);
        } else {
            void toggleFavoriteStreamer(id, favoriteMetaOf(stream, id));
        }
    }, []);

    const { visible, width, showExpanded, isOverlay } = calculateSidebarState();

    // Expand the panel unblurred (cheap) and only fade the frosted glass in once
    // it has finished widening. Blurring an element while its width animates forces
    // a per-frame backdrop re-raster, which is what made the expand choppy. Gating
    // the blur behind the resize keeps the expand/collapse at full frame rate, then
    // the glass settles in with a soft fade.
    useEffect(() => {
        if (showExpanded) {
            const t = setTimeout(() => setBlurReady(true), SIDEBAR_BLUR_SETTLE_DELAY);
            return () => clearTimeout(t);
        }
        setBlurReady(false);
    }, [showExpanded]);

    // Live channels followed on other platforms. The backend poller keeps this
    // fresh via `provider-live-update` (subscribed once in App), so the sidebar
    // just reads the snapshot.
    const providerFollowsLive = useFollowsStore((s) => s.liveByKey);
    // Live rows for favourites the backend sweeps because they're followed
    // nowhere. Merged with the follow sources below, never shown on its own.
    const favoritesLive = useFavoritesStore((s) => s.liveByKey);
    // Composite keys the user subscribes to, imported alongside the follow list.
    const providerFollows = useFollowsStore((s) => s.follows);

    // The other platforms' directories. On a single platform this is that
    // platform's ranked list, standing in for Twitch's recommendations. On
    // `all` it is EVERY non-Twitch platform's, which then gets merged with the
    // Twitch picks below: the second section used to stay Twitch-only there,
    // so the sidebar said "all platforms" while recommending one.
    useEffect(() => {
        if (activePlatform === 'twitch') {
            setPlatformTopLive([]);
            return;
        }
        const onAllPlatforms = activePlatform === 'all';
        const targets = onAllPlatforms
            ? WATCHABLE_PROVIDERS.filter((p) => p !== 'twitch')
            : [activePlatform];
        let cancelled = false;
        // Settled per platform, not all-or-nothing: one platform being down
        // must not empty the section for the others.
        Promise.all(
            targets.map((provider) =>
                invoke<{ streams: TwitchStream[] }>('provider_directory', {
                    provider,
                    // One ranked page: the sorted endpoint has no cursor, so a
                    // short list could never grow. Smaller when merging, since
                    // the Twitch picks are carrying most of the section.
                    limit: onAllPlatforms ? 25 : 50,
                })
                    .then((page) => page.streams ?? [])
                    .catch(() => [] as TwitchStream[]),
            ),
        ).then((pages) => {
            if (!cancelled) setPlatformTopLive(pages.flat());
        });
        return () => {
            cancelled = true;
        };
    }, [activePlatform]);
    const subscribedKeys = useMemo(
        () =>
            new Set(
                providerFollows
                    .filter((f) => f.subscribed)
                    .map((f) => `${f.provider}:${f.channel}`),
            ),
        [providerFollows],
    );

    // If sidebar is completely disabled, render nothing
    if (sidebarMode === 'disabled') {
        return null;
    }

    // Split followed (live) channels into Favorites and the rest, so each gets
    // its own labeled section — mirroring how Followed is separated from
    // Recommended. The sidebar only ever lists live channels, so these are the
    // live favorites vs. the live non-favorite follows.
    // The sidebar answers "who can I watch" for the platform the app is scoped
    // to. On `all` it merges everything, sorted by viewers rather than grouped
    // by service; on a platform it becomes that platform's sidebar entirely.
    const onTwitch = activePlatform === 'twitch';
    const onAll = activePlatform === 'all';
    const providerLiveStreams = Object.values(providerFollowsLive)
        .filter((row) => row.is_live && (onAll || row.provider === activePlatform))
        .sort((a, b) => b.viewer_count - a.viewer_count);

    const twitchFollowed = onAll || onTwitch ? followedStreams : [];

    // Live favourites, from all three places a live row can come from: your
    // Twitch follows, the provider follow poller, and the favourites sweep
    // (which covers channels you follow nowhere and is the whole reason a
    // favourite works without a follow).
    //
    // Deduped on the FAVOURITE id, not `streamKey`: on YouTube the same channel
    // arrives keyed by video id from a browse row and by UC id from a live
    // check, and a streamKey dedupe would list it twice.
    const favoriteLiveRows = Object.values(favoritesLive).filter(
        (row) => row.is_live && (onAll || row.provider === activePlatform),
    );
    const isFavoriteRow = (s: TwitchStream) => {
        const id = favoriteIdOf(s);
        return !!id && isFavoriteStreamer(id);
    };
    const favoriteStreams = dedupeByFavoriteId([
        ...twitchFollowed,
        ...providerLiveStreams,
        ...favoriteLiveRows,
    ])
        .filter(isFavoriteRow)
        .sort((a, b) => (b.viewer_count ?? 0) - (a.viewer_count ?? 0));

    // Everything already shown under Favourites is excluded below, provider rows
    // included: that exclusion used to cover only Twitch, so a favourited Kick
    // channel appeared in both sections.
    const favoriteShownKeys = new Set(favoriteStreams.map((s) => streamKey(s)));
    const followedNonFavoriteStreams = [
        ...twitchFollowed,
        ...providerLiveStreams,
    ].filter((s) => !favoriteShownKeys.has(streamKey(s)));

    // Second section: Twitch has real personalized recommendations; the other
    // platforms don't, so they show their viewer-ranked directory instead. The
    // label changes with it rather than calling a directory "Recommended".
    //
    // On `all` the two are merged and ranked together, the same way the Followed
    // section above merges. Leaving it as Twitch's picks alone was the sidebar
    // claiming every platform in one section and exactly one in the next.
    const rawSecondSection = onAll
        ? {
              label: 'Recommended',
              streams: [...recommendedStreams, ...platformTopLive].sort(
                  (a, b) => (b.viewer_count ?? 0) - (a.viewer_count ?? 0),
              ),
          }
        : onTwitch
          ? { label: 'Recommended', streams: recommendedStreams }
          : { label: 'Top live', streams: platformTopLive };

    // Nothing already listed above gets recommended below it. Twitch's own
    // recommendations already exclude your follows, but a platform DIRECTORY is
    // just "who is live, ranked", so a Kick channel you follow was appearing in
    // Followed and again three rows down. Deduping against what is actually
    // rendered rather than against the follow list also covers a favourite,
    // which is the same channel in a third section.
    const shownKeys = new Set(
        [...favoriteStreams, ...followedNonFavoriteStreams].map((s) => streamKey(s)),
    );
    const secondSection = {
        label: rawSecondSection.label,
        streams: rawSecondSection.streams.filter((s) => !shownKeys.has(streamKey(s))),
    };

    // Section-presence flags drive both the headers and the dividers between them.
    // No longer gated on Twitch or on being signed in: a favourited Kick or
    // YouTube channel is real without a Twitch login, and a favourite you don't
    // follow is real without being in any follow list.
    const hasFavorites = favoriteStreams.length > 0;
    // Only Twitch's list needs a Twitch login; the others browse signed out.
    const hasFollowed = followedNonFavoriteStreams.length > 0 && (isAuthenticated || providerLiveStreams.length > 0);
    const hasRecommended = showRecommended && secondSection.streams.length > 0;

    // Shared row renderer so Favorites / Followed / Recommended stay identical.
    const renderStreamItem = (stream: TwitchStream, showFavorite: boolean) => (
        <StreamItem
            // Composite provider:channel key. Bare platform ids collide across
            // platforms in the mixed list, and provider rows may carry none.
            key={streamKey(stream)}
            stream={stream}
            showFavorite={showFavorite}
            showExpanded={showExpanded}
            isCurrentStream={currentStream?.user_login === stream.user_login}
            isFavorite={(() => { const id = favoriteIdOf(stream); return !!id && isFavoriteStreamer(id); })()}
            isSubscribed={subscribedKeys.has(streamKey(stream))}
            showPlatformBadge={onAll}
            hasDrops={stream.game_name ? dropsGameNames.has(stream.game_name.toLowerCase()) : false}
            hypeTrainStatus={activeHypeTrainChannels.get(stream.user_id)}
            watchStreak={watchStreaks[stream.user_id] ?? 0}
            isHeartAnimating={(() => { const id = favoriteIdOf(stream); return !!id && animatingHearts.has(id); })()}
            profileImage={getProfileImage(stream)}
            onStreamClick={handleStreamClick}
            onFavoriteClick={handleFavoriteClick}
        />
    );

    return (
        <>
            {/* SVG Definitions for Liquid Glass Heart */}
            <svg width="0" height="0" className="absolute pointer-events-none">
                <defs>
                    <linearGradient id="glass-heart-fill" x1="0" y1="0" x2="0" y2="1">
                        <stop offset="0%" stopColor="rgba(255, 255, 255, 0.4)" />
                        <stop offset="30%" stopColor="rgba(236, 72, 153, 0.2)" />
                        <stop offset="100%" stopColor="rgba(236, 72, 153, 0.6)" />
                    </linearGradient>
                    <linearGradient id="glass-heart-stroke" x1="0" y1="0" x2="0" y2="1">
                        <stop offset="0%" stopColor="rgba(255, 255, 255, 0.8)" />
                        <stop offset="100%" stopColor="rgba(255, 255, 255, 0.1)" />
                    </linearGradient>
                </defs>
            </svg>

            {/* Edge trigger zone for hidden mode. z-[50] sits above Home's
                full-screen overlay (z-40) so hover-to-reveal works in home
                view, but stays below modal dialogs (which render later in DOM). */}
            {sidebarMode === 'hidden' && !visible && (
                <div
                    ref={edgeTriggerRef}
                    className={`fixed ${onRight ? 'right-0' : 'left-0'} top-0 h-full z-50`}
                    style={{ width: HIDDEN_TRIGGER_ZONE }}
                    onMouseEnter={() => setIsEdgeHovered(true)}
                />
            )}

            {/* Spacer for compact overlay mode to maintain layout - only when in overlay mode */}
            {sidebarMode === 'compact' && expandOnHover && isOverlay && (
                <div style={{ width: COMPACT_WIDTH, minWidth: COMPACT_WIDTH, flexShrink: 0, order: onRight ? 1 : 0 }} />
            )}

            {/* Main sidebar */}
            <div
                ref={sidebarRef}
                className={`
                    ${onRight ? 'border-l' : 'border-r'} border-borderSubtle flex flex-col flex-shrink-0
                    transition-[width,min-width,opacity,transform] duration-200 ease-[cubic-bezier(0.4,0,0.2,1)]
                    ${isOverlay
                        // Overlay panels are position:fixed, so a percentage height
                        // resolves against the viewport. Anchoring top (below the
                        // 40px title bar) AND bottom makes the panel exactly as tall
                        // as the content area — a full `h-full` shoved down would
                        // resolve to 100vh and hide the bottom of the scroll list
                        // off-screen at every non-fullscreen height. The top offset
                        // MUST track the title bar height (h-[40px] in TitleBar.tsx);
                        // when it lagged at top-8 the panel rode up under the bar and
                        // its frosted backing clipped the title-bar action icons.
                        ? `fixed ${onRight ? 'right-0' : 'left-0'} top-10 bottom-0 z-50`
                        : 'relative h-full'
                    }
                `}
                style={{
                    width: width,
                    minWidth: isOverlay ? 0 : width,
                    opacity: visible ? 1 : 0,
                    pointerEvents: visible ? 'auto' : 'none',
                    transform: visible ? 'translateX(0)' : `translateX(${onRight ? '10px' : '-10px'})`,
                    order: onRight ? 1 : 0,
                }}
                onMouseEnter={() => {
                    // Cancel any pending close timeout
                    if (closeTimeoutRef.current) {
                        clearTimeout(closeTimeoutRef.current);
                        closeTimeoutRef.current = null;
                    }
                    setIsHovered(true);
                }}
                onMouseLeave={() => {
                    // Don't close while resizing
                    if (isResizing) return;

                    // Add a delay before closing in hidden mode or compact overlay mode
                    const shouldDelay = sidebarMode === 'hidden' || (sidebarMode === 'compact' && expandOnHover);
                    if (shouldDelay) {
                        closeTimeoutRef.current = setTimeout(() => {
                            setIsHovered(false);
                            setIsEdgeHovered(false);
                            closeTimeoutRef.current = null;
                        }, SIDEBAR_CLOSE_DELAY);
                    } else {
                        setIsHovered(false);
                        setIsEdgeHovered(false);
                    }
                }}
            >
                {/* Background layers, kept separate from the content so the costly
                    frosted blur can fade in independently of the width change. While
                    the panel is widening only the cheap solid backing shows; once the
                    expand has settled (blurReady) the glass blur fades in, so the blur
                    is never re-rasterized mid-resize. These sit at z-0; the content
                    below is lifted to z-10 so it always paints above them. */}
                {/* Glass fades in FIRST, over the still-opaque solid, then the solid
                    fades out (200ms delay) to reveal the translucency. Sequencing them
                    this way keeps the panel fully opaque until the glass is fully in, so
                    it never dips more transparent than its final state mid-transition. */}
                <div
                    aria-hidden
                    className="absolute inset-0 z-0 bg-tertiary pointer-events-none"
                    style={{
                        opacity: blurReady ? 0 : 1,
                        transition: `opacity 260ms ease-out ${blurReady ? '200ms' : '0ms'}`,
                    }}
                />
                <div
                    aria-hidden
                    className="absolute inset-0 z-0 transition-opacity duration-200 ease-out pointer-events-none"
                    style={{
                        opacity: blurReady ? 1 : 0,
                        // Glassiness-aware frosted backing — tracks the global
                        // Glassiness slider in lockstep with every other glass
                        // surface. Same two-part recipe as .glass-panel: an opaque
                        // tertiary base fades IN as --glass-strength drops toward 0
                        // (so the panel goes fully solid at 0%), with the signature
                        // dark tint always layered on top via background-image.
                        backgroundColor: 'color-mix(in srgb, var(--color-background-tertiary) calc((1 - var(--glass-strength)) * 100%), transparent)',
                        backgroundImage: 'linear-gradient(rgba(26, 26, 27, 0.75), rgba(26, 26, 27, 0.75))',
                        // Frost scales with the slider; the html[data-glass="off"]
                        // floor hard-strips any remaining blur at 0%.
                        backdropFilter: blurReady ? 'blur(calc(24px * var(--glass-strength)))' : undefined,
                        WebkitBackdropFilter: blurReady ? 'blur(calc(24px * var(--glass-strength)))' : undefined,
                    }}
                />

                {/* Resize handle - only show when expanded */}
                {showExpanded && (
                    <Tooltip content="Drag to resize sidebar" delay={500} side="right">
                        <div
                            className={`
                                absolute ${onRight ? 'left-0' : 'right-0'} top-0 w-1 h-full cursor-ew-resize z-10
                                hover:bg-accent/50 active:bg-accent transition-colors
                                ${isResizing ? 'bg-accent' : 'bg-transparent'}
                            `}
                            onMouseDown={handleResizeStart}
                        />
                    </Tooltip>
                )}

                {/* Header — only rendered when it actually carries something: the
                    "Streams" label (expanded) or the manual collapse/expand toggle
                    (compact with expand-on-hover OFF). In the collapsed
                    expand-on-hover state it would hold neither, so rendering it then
                    just leaves an empty padded, bordered bar below the title bar. */}
                {(showExpanded || (sidebarMode === 'compact' && !expandOnHover)) && (
                    <div className={`
                        relative z-10 flex items-center p-2 border-b border-borderSubtle
                        ${showExpanded ? 'justify-between' : 'justify-center'}
                    `}>
                        {showExpanded && (
                            <span className="text-sm font-semibold text-textPrimary">Streams</span>
                        )}
                        {sidebarMode === 'compact' && !expandOnHover && (
                            <Tooltip content={isManuallyExpanded ? 'Collapse sidebar' : 'Expand sidebar'} delay={200} side="right">
                                <button
                                    onClick={() => setIsManuallyExpanded(!isManuallyExpanded)}
                                    className="p-1.5 rounded hover:bg-surface-hover text-textSecondary hover:text-textPrimary transition-all"
                                >
                                    {isManuallyExpanded ? <ChevronLeft size={16} /> : <ChevronRight size={16} />}
                                </button>
                            </Tooltip>
                        )}
                    </div>
                )}

                {/* Scrollable stream list */}
                <div
                    ref={scrollContainerRef}
                    className={`relative z-10 flex-1 min-h-0 overflow-x-hidden py-1 ${
                        // Hide scrollbar in compact mode with expand-on-hover when not expanded
                        sidebarMode === 'compact' && expandOnHover && !showExpanded
                            ? 'overflow-y-hidden'
                            : 'overflow-y-auto scrollbar-thin'
                        }`}
                >
                    {/* Favorites Section — favorited live channels, pulled out of
                        Followed into their own labeled group. */}
                    {/* Favorites get their own shelf: a faint wash of the heart's
                        pink plus a hairline, so the personal watchlist reads as a
                        distinct block instead of one more section in the same list.
                        Deliberately quiet - the wash is 4 percent - and it replaces
                        the old divider, which a bordered shelf makes redundant. */}
                    {hasFavorites && (
                        <div
                            className="mb-2 rounded-lg pb-1"
                            style={{
                                background: 'color-mix(in srgb, var(--color-highlight-pink) 2%, transparent)',
                                border: '1px solid color-mix(in srgb, var(--color-highlight-pink) 8%, transparent)',
                            }}
                        >
                            <SectionHeader
                                icon={Heart}
                                label="Favorites"
                                count={favoriteStreams.length}
                                showExpanded={showExpanded}
                                iconNode={
                                    <Heart
                                        size={16}
                                        fill="url(#glass-heart-fill)"
                                        stroke="url(#glass-heart-stroke)"
                                        strokeWidth={1.5}
                                        className="flex-shrink-0 drop-shadow-[0_4px_8px_color-mix(in_srgb,var(--color-highlight-pink)_50%,transparent)]"
                                    />
                                }
                            />
                            <div className="space-y-0.5 px-1">
                                {favoriteStreams.map(stream => renderStreamItem(stream, true))}
                            </div>
                        </div>
                    )}

                    {/* Followed Streams Section — live follows that aren't favorited. */}
                    {hasFollowed && (
                        <div className="mb-2">
                            <SectionHeader icon={Users} label="Followed" count={followedNonFavoriteStreams.length} showExpanded={showExpanded} />
                            <div className="space-y-0.5">
                                {followedNonFavoriteStreams.map(stream => renderStreamItem(stream, true))}
                            </div>
                        </div>
                    )}

                    {/* Divider before Recommended */}
                    {hasRecommended && (hasFavorites || hasFollowed) && (
                        <div className="mx-2 my-2 border-t border-borderSubtle" />
                    )}

                    {/* Recommended Streams Section */}
                    {hasRecommended && (
                        <div>
                            <SectionHeader icon={Sparkles} label={secondSection.label} count={secondSection.streams.length} showExpanded={showExpanded} />
                            <div className="space-y-0.5">
                                {secondSection.streams.map(stream => renderStreamItem(stream, false))}
                            </div>

                            {/* Loading more indicator. Infinite scroll is Twitch's
                                paginated recommendations; the platform directory
                                is a single ranked page. */}
                            {isLoadingMore && (onAll || onTwitch) && (
                                <div className="flex justify-center py-2">
                                    <div className="animate-spin rounded-full h-5 w-5 border-2 border-borderSubtle border-t-accent" />
                                </div>
                            )}
                        </div>
                    )}

                    {/* Empty state. Names the platform the sidebar is actually
                        scoped to — it used to say "log in" on a Kick or YouTube
                        view, where a Twitch login would change nothing. */}
                    {!isAuthenticated && !hasRecommended && (
                        <div className={`
                            flex items-center justify-center text-center p-4
                            ${showExpanded ? '' : 'flex-col'}
                        `}>
                            {showExpanded ? (
                                <p className="text-xs text-textMuted">
                                    {activePlatform === 'all' || activePlatform === 'twitch'
                                        ? 'Log in to see followed streams'
                                        : `Connect ${providerLabel(activePlatform as ProviderId)} to see followed streams`}
                                </p>
                            ) : (
                                <Users size={16} className="text-textMuted" />
                            )}
                        </div>
                    )}
                </div>
            </div>

            {/* Spacer for hidden mode to maintain layout */}
            {sidebarMode === 'hidden' && (
                <div style={{ width: 0, minWidth: 0, flexShrink: 0 }} />
            )}
        </>
    );
};

export default Sidebar;
