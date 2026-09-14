import { useEffect, useLayoutEffect, useState, useRef, useCallback, useMemo } from 'react';
import { useShallow } from 'zustand/react/shallow';
import { useAppStore, ensureHomeSnapshotSync, clipSourceOf, HomeTab } from '../stores/AppStore';
import { createPortal } from 'react-dom';
import { Search, ArrowLeft, Heart, X, Gift, Pickaxe, LayoutGrid, Flame, ArrowUpRight, Undo2, Users, User, Loader2, Clock, Play, Check, Plus } from 'lucide-react';
import { MediaCard } from './MediaCard';
import ContinueWatchingRow from './ContinueWatchingRow';
import { formatCardDate, mediaKindOfVideo, videoDurationLabel, vodThumbUrl, VOD_FALLBACK_THUMB } from '../utils/vodProgress';
import { motion, LayoutGroup, AnimatePresence } from 'framer-motion';
import { usemultiNookStore } from '../stores/multiNookStore';

import { invoke } from '@tauri-apps/api/core';
import type { TwitchStream, TwitchCategory, CategoryInfo, TwitchClip, TwitchVideo } from '../types';
import LoadingWidget from './LoadingWidget';
import StreamTitleWithEmojis from './StreamTitleWithEmojis';
import { StreamTileTags } from './StreamTileTags';
import { useContextMenuStore } from '../stores/contextMenuStore';
import { Tooltip } from './ui/Tooltip';
import { ProviderLogo } from './ProviderLogo';
import { usePlatformAccountStore } from '../stores/platformAccountStore';
import { PlatformLoginButton } from './PlatformLoginButton';
import { WATCHABLE_PROVIDERS, PROVIDER_WATCH, providerLabel, type ProviderId, type ProviderCategory } from '../types/providers';
import { useFollowsStore } from '../stores/followsStore';
import { useFavoritesStore } from '../stores/favoritesStore';
import { favoriteIdOf, favoriteMetaOf, dedupeByFavoriteId } from '../utils/favorites';
import { streamProvider, streamKey, followIdentifier } from '../utils/streamProvider';
import { makeKey } from '../utils/providerKey';

import { GlassSelect } from './ui/GlassSelect';
import { CategorySearchBox } from './ui/CategorySearchBox';
import {
    loadRecentSearches,
    addRecentSearch,
    removeRecentSearch,
    clearRecentSearches,
    type RecentSearch,
    type SearchMode,
} from '../utils/searchHistory';

import { formatViewerCount } from '../utils/streamStats';
import { useStreamAvatars } from '../hooks/useStreamAvatars';
import { useVisibleAvatarKeys } from '../hooks/useVisibleAvatarKeys';
import { Logger } from '../utils/logger';
import { useVisibleInterval } from '../utils/useVisibleInterval';
// Types for drops data
interface DropCampaign {
    id: string;
    name: string;
    game_id: string;
    game_name: string;
}

/**
 * Vertical falloff for the Favorites shelf's film grain.
 *
 * VERTICAL only, and that is the point. A pink wash used to share this shape and
 * it went through three attempts as a centred radial, all of which failed for
 * one structural reason: the shelf is a full-bleed row whose cards are
 * LEFT-ALIGNED, so a dome centred at 50% puts its peak in empty space and its
 * tail over the cards - the more screen you give it, the less it reads. Every
 * fix then oscillated between "bright enough to see in the middle" (still
 * non-zero where its box ends, so it clipped square into a hard line) and
 * "terminates inside the box" (invisible on a maximized window). Those were not
 * two bugs; they were one wrong shape. The wash was cut entirely afterwards
 * (Brandon: "not a fan of the pink wash, lets just keep the grain"), but the
 * band survives it and the reasoning is worth keeping for the next decoration.
 *
 * A band has no horizontal falloff, so nothing can clip it and it can be
 * extended sideways freely; it is uniform at any monitor width; and it is zero
 * at both ends BY CONSTRUCTION, so it cannot grow an edge from a change in the
 * section's height or from the scroll container clipping the bleed above it.
 */
const FAVORITES_GRAIN_MASK =
  'linear-gradient(to bottom, transparent 0%, white 38%, white 62%, transparent 100%)';

const FlyingDot = ({ startX, startY, targetX, targetY }: { startX: number, startY: number, targetX: number, targetY: number }) => {
    const [isFlying, setIsFlying] = useState(false);

    useEffect(() => {
        const timer = setTimeout(() => setIsFlying(true), 10);
        return () => clearTimeout(timer);
    }, []);

    return (
        <div 
            className="fixed z-[9999] pointer-events-none flex h-5 w-5 items-center justify-center rounded-full bg-red-500 shadow-[0_0_15px_color-mix(in_srgb,var(--color-live)_80%,transparent)]"
            style={{
                left: isFlying ? targetX : startX,
                top: isFlying ? targetY : startY,
                opacity: isFlying ? 0.3 : 1,
                transform: isFlying ? 'scale(0.3)' : 'scale(1)',
                transition: 'all 500ms cubic-bezier(0.25, 1, 0.5, 1)'
            }}
        >
           <LayoutGrid size={12} className="text-white" />
        </div>
    );
};

const ReverseFlyingDot = ({ startX, startY, targetX, targetY }: { startX: number, startY: number, targetX: number, targetY: number }) => {
    const [isFlying, setIsFlying] = useState(false);

    useEffect(() => {
        const timer = setTimeout(() => setIsFlying(true), 10);
        return () => clearTimeout(timer);
    }, []);

    return (
        <div 
            className="fixed z-[9999] pointer-events-none flex h-5 w-5 items-center justify-center rounded-full bg-accent shadow-[0_0_15px_rgba(var(--color-accent-rgb),0.8)]"
            style={{
                left: isFlying ? targetX : startX,
                top: isFlying ? targetY : startY,
                opacity: isFlying ? 1 : 0.3,
                transform: isFlying ? 'scale(1)' : 'scale(0.3)',
                transition: 'all 500ms cubic-bezier(0.25, 1, 0.5, 1)'
            }}
        >
           <Undo2 size={10} className="text-white" />
        </div>
    );
};

const MultiNookToggle = () => {
    const { isMultiNookActive, toggleMultiNook, slots, flyingAnimation, recallAnimation } = usemultiNookStore();
    // Action only, stable for the store's lifetime: no subscription needed.
    const { toggleHome } = useAppStore.getState();
    const [animateBadge, setAnimateBadge] = useState(false);
    const buttonRef = useRef<HTMLButtonElement>(null);
    const [flyingDots, setFlyingDots] = useState<Array<{ id: number, startX: number, startY: number, targetX: number, targetY: number }>>([]);
    const [reverseDots, setReverseDots] = useState<Array<{ id: number, startX: number, startY: number, targetX: number, targetY: number }>>([]);

    useEffect(() => {
        if (flyingAnimation && buttonRef.current) {
            const rect = buttonRef.current.getBoundingClientRect();
            // Target is the top right of the button where the badge will sit
            const targetX = rect.right - 10;
            const targetY = rect.top - 5;
            
            const newDot = {
                id: flyingAnimation.id,
                startX: flyingAnimation.x,
                startY: flyingAnimation.y,
                targetX,
                targetY
            };
            
            setFlyingDots(prev => [...prev, newDot]);
            
            setTimeout(() => {
                setFlyingDots(prev => prev.filter(d => d.id !== newDot.id));
                setAnimateBadge(true);
                setTimeout(() => setAnimateBadge(false), 200);
            }, 500); 
        }
    }, [flyingAnimation]);

    // Handle reverse recall animation — dot flies from badge to card
    useEffect(() => {
        if (recallAnimation) {
            const newDot = {
                id: recallAnimation.id,
                startX: recallAnimation.sourceX,
                startY: recallAnimation.sourceY,
                targetX: recallAnimation.targetX,
                targetY: recallAnimation.targetY,
            };
            
            queueMicrotask(() => setReverseDots(prev => [...prev, newDot]));
            
            setTimeout(() => {
                setReverseDots(prev => prev.filter(d => d.id !== newDot.id));
            }, 600);
        }
    }, [recallAnimation]);

    return (
        <>
            <Tooltip content={isMultiNookActive ? 'Return to MultiNook' : 'Enter MultiNook'} side="bottom">
                <button
                    id="multinook-return-button"
                    ref={buttonRef}
                    onClick={() => {
                        if (isMultiNookActive) {
                            toggleHome();
                        } else {
                            toggleMultiNook();
                        }
                    }}
                    className={`relative px-3 py-1.5 text-sm font-medium rounded-lg transition-all whitespace-nowrap mr-0.5 ${
                        isMultiNookActive
                            ? 'glass-button text-accent shadow-[0_0_15px_rgba(var(--color-accent-rgb),0.3)]'
                            : 'text-textSecondary hover:text-textPrimary'
                    }`}
                >
                    {isMultiNookActive ? 'Return' : 'MultiNook'}
                    {!isMultiNookActive && slots.length > 0 && (
                        <span 
                            className={`absolute -top-1.5 -right-1.5 flex h-4 w-4 items-center justify-center rounded-full glass-button !bg-error/30 text-[9px] font-extrabold text-white !shadow-[0_2px_10px_color-mix(in_srgb,var(--color-error)_40%,transparent),inset_0_1px_rgba(255,255,255,0.2)] z-50 ${
                                animateBadge ? 'scale-150 ring-2 ring-error transition-transform duration-200' : 'scale-100 transition-transform duration-500'
                            }`}
                        >
                            {slots.length}
                        </span>
                    )}
                </button>
            </Tooltip>
            {flyingDots.length > 0 && typeof document !== 'undefined' && createPortal(
                flyingDots.map(dot => (
                    <FlyingDot key={dot.id} {...dot} />
                )),
                document.body
            )}
            {reverseDots.length > 0 && typeof document !== 'undefined' && createPortal(
                reverseDots.map(dot => (
                    <ReverseFlyingDot key={dot.id} {...dot} />
                )),
                document.body
            )}
        </>
    );
};

// Top-nav entry point for the MultiChat popout, sitting beside MultiNook so the
// two "multi" workspaces live together. Clicking opens the popout window — or
// focuses it if one's already open — via openMultiChatWindow with no channel
// (which seeds an empty window, or restores its previous tabs). The count badge
// mirrors MultiNook's: it shows how many channels are currently popped out,
// which main tracks through the `channelsInPopouts` aggregate the popout
// broadcasts. No outer glow on the active state (no-glow aesthetic); the badge
// uses the approved soft-shadow notification recipe.
const MultiChatButton = () => {
    const channelsInPopouts = useAppStore((s) => s.channelsInPopouts);
    const count = channelsInPopouts.size;
    const active = count > 0;

    const handleOpen = async () => {
        try {
            const { openMultiChatWindow } = await import('../utils/multichatWindow');
            await openMultiChatWindow({});
        } catch (err) {
            Logger.error('[Home] openMultiChatWindow failed:', err);
        }
    };

    return (
        <Tooltip content="Open MultiChat" side="bottom">
            <button
                onClick={handleOpen}
                className={`relative px-3 py-1.5 text-sm font-medium rounded-lg transition-all whitespace-nowrap mr-0.5 ${
                    active
                        ? 'glass-button text-accent'
                        : 'text-textSecondary hover:text-textPrimary'
                }`}
            >
                MultiChat
                {count > 0 && (
                    <span className="absolute -top-1.5 -right-1.5 flex h-4 w-4 items-center justify-center rounded-full glass-button !bg-accent/30 text-[9px] font-extrabold text-white !shadow-[0_2px_10px_rgba(var(--color-accent-rgb),0.35),inset_0_1px_rgba(255,255,255,0.2)] z-50">
                        {count}
                    </span>
                )}
            </button>
        </Tooltip>
    );
};

const QuickAddButton = ({ stream }: { stream: TwitchStream }) => {
    const { addSlot, slots, triggerAddAnimation } = usemultiNookStore();
    const [rotation, setRotation] = useState(0); 
    const buttonRef = useRef<HTMLButtonElement>(null);

    // Compute the dynamic rotation pointing toward the return button
    const handleHover = () => {
        if (!buttonRef.current) return;
        const targetBtn = document.getElementById('multinook-return-button');
        if (targetBtn) {
            const targetRect = targetBtn.getBoundingClientRect();
            const sourceRect = buttonRef.current.getBoundingClientRect();
            
            const targetX = targetRect.left + (targetRect.width / 2);
            const targetY = targetRect.top + (targetRect.height / 2);
            const sourceX = sourceRect.left + (sourceRect.width / 2);
            const sourceY = sourceRect.top + (sourceRect.height / 2);
            
            const angle = Math.atan2(targetY - sourceY, targetX - sourceX) * (180 / Math.PI);
            // ArrowUpRight points initially to top right (-45 Cartesian). Offset sets it naturally.
            setRotation(angle + 45);
        }
    };

    useEffect(() => {
        handleHover(); // Initial calculation

        // Attach a listener to the parent card so angle calculates perfectly when the card is hovered
        const groupAncestor = buttonRef.current?.closest('.group');
        if (groupAncestor) {
            groupAncestor.addEventListener('mouseenter', handleHover);
            return () => groupAncestor.removeEventListener('mouseenter', handleHover);
        }
    }, []);

    // Also update on resize to ensure arrow points perfectly in different window dimensions
    useEffect(() => {
        const resizeListener = () => handleHover();
        window.addEventListener('resize', resizeListener);
        return () => window.removeEventListener('resize', resizeListener);
    }, []);

    // Composite compare: a Twitch tile of this name must not hide the add button
    // on the Kick row of the same name, and vice versa.
    const rowKey = makeKey(streamProvider(stream), stream.user_login);
    if (slots.some((s) => makeKey(s.provider ?? 'twitch', s.channelLogin) === rowKey)) return null;

    return (
        <div 
            className="absolute -top-2.5 -right-2.5 z-20 opacity-0 group-hover:opacity-100 transition-all duration-300 scale-90 group-hover:scale-100 group-hover:-translate-y-1 group-hover:translate-x-1"
            onMouseEnter={handleHover}    
        >
            <Tooltip content="Add to MultiNook" side="top">
                <button
                    ref={buttonRef}
                    onClick={(e) => {
                        e.stopPropagation();
                        triggerAddAnimation(e.clientX, e.clientY, stream.user_login, streamProvider(stream));
                        addSlot(stream.user_login, streamProvider(stream));
                    }}
                    className="flex items-center justify-center glass-button !rounded-full aspect-square !p-1.5 text-white shadow-[0_4px_10px_rgba(0,0,0,0.5)]"
                >
                    <ArrowUpRight 
                        size={14} 
                        strokeWidth={2} 
                        style={{ transform: `rotate(${rotation}deg)`, transition: 'transform 0.4s cubic-bezier(0.34, 1.56, 0.64, 1)' }}
                    />
                </button>
            </Tooltip>
        </div>
    );
};



const Home = () => {
    // Actions without a subscription (stable for the store's lifetime); state
    // through one shallow-compared selector below, so unrelated store writes
    // (toasts, viewer-count updates, hype-train ticks on other surfaces) no
    // longer re-render all of Home.
    const {
        loadMoreRecommendedStreams,
        startStream,
        toggleFavoriteStreamer,
        isFavoriteStreamer,
        loginToTwitch,
        setHomeActiveTab,
        setHomeSelectedCategory,
        setSearchReturnTab,
        setCachedTopGames,
        appendCachedTopGames,
        setProfileModalUser,
        openDropsWithSearch,
        playMedia,
        setHomeCategoryTab,
        setClipsPeriod,
        setVideosSort,
        setVideosPeriod,
        setMediaSearchQuery,
    } = useAppStore.getState();
    const {
        followedStreams,
        recommendedStreams,
        hasMoreRecommended,
        isLoadingMore,
        isAuthenticated,
        isLoading,
        streamUrl,
        // Navigation state from AppStore
        homeActiveTab,
        homeSelectedCategory,
        searchReturnTab,
        // Category cache
        cachedTopGames,
        cachedGamesCursor,
        cachedHasMoreGames,
        cachedTopGamesTimestamp,
        // Hype Train status for stream badges
        activeHypeTrainChannels,
        watchStreaks,
        offlineFollowedChannels,
        homeCategoryTab,
        clipsPeriod,
        videosSort,
        videosPeriod,
        mediaSearchQuery,
        // Destructured now (it used to be tracked only for re-render): the live
        // favourites list derives from it, so the memo needs it as a dependency.
        favoriteStreamers,
        favoriteChannels,
    } = useAppStore(
        useShallow((s) => ({
            followedStreams: s.followedStreams,
            recommendedStreams: s.recommendedStreams,
            hasMoreRecommended: s.hasMoreRecommended,
            isLoadingMore: s.isLoadingMore,
            isAuthenticated: s.isAuthenticated,
            isLoading: s.isLoading,
            streamUrl: s.streamUrl,
            homeActiveTab: s.homeActiveTab,
            homeSelectedCategory: s.homeSelectedCategory,
            searchReturnTab: s.searchReturnTab,
            cachedTopGames: s.cachedTopGames,
            cachedGamesCursor: s.cachedGamesCursor,
            cachedHasMoreGames: s.cachedHasMoreGames,
            cachedTopGamesTimestamp: s.cachedTopGamesTimestamp,
            activeHypeTrainChannels: s.activeHypeTrainChannels,
            watchStreaks: s.watchStreaks,
            offlineFollowedChannels: s.offlineFollowedChannels,
            homeCategoryTab: s.homeCategoryTab,
            clipsPeriod: s.clipsPeriod,
            videosSort: s.videosSort,
            videosPeriod: s.videosPeriod,
            mediaSearchQuery: s.mediaSearchQuery,
            // `isFavoriteStreamer` is called during render (the favourite sort
            // and the hearts) and reads settings.favorite_streamers, which isn't
            // itself reactive, so this slice is what re-renders on a toggle.
            favoriteStreamers: s.settings.favorite_streamers,
            // Identity for favourites, so the offline roster can draw a channel
            // you don't follow anywhere.
            favoriteChannels: s.settings.favorite_channels,
        })),
    );
    const externalDropsProvider = useAppStore((s) => s.externalDropsProvider);

    // The offline roster and its last-broadcast times come from the Rust Home
    // snapshot (store fields, refreshed by Rust every 10 min and on mount when
    // stale), so they survive Home being unmounted and cost nothing to reopen.
    const offlineLastBroadcasts = useAppStore((s) => s.offlineLastBroadcasts);
    const offlineFollowsAt = useAppStore((s) => s.offlineFollowsAt);
    const isLoadingOfflineChannels = isAuthenticated && offlineFollowsAt === null;

    // Fill in identity for favourites saved as bare ids, before the sidecar
    // existed. Without a name and a face those can't be drawn in the offline
    // roster at all. One batched lookup, and it stops matching once filled.
    useEffect(() => {
        if (homeActiveTab !== 'following' || !isAuthenticated) return;
        void useAppStore.getState().backfillFavoriteIdentities();
    }, [homeActiveTab, isAuthenticated]);


    // MultiNook ghost card state
    const multiNookSlots = usemultiNookStore(s => s.slots);
    const isMultiNookActive = usemultiNookStore(s => s.isMultiNookActive);
    const suckUpKey = usemultiNookStore(s => s.suckUpKey);
    const materializingKey = usemultiNookStore(s => s.materializingKey);
    const triggerRecallAnimation = usemultiNookStore(s => s.triggerRecallAnimation);
    
    // Determine if Home is acting as an overlay over a playing stream/multinook
    const isOverlayMode = !!streamUrl || isMultiNookActive;
    // Compared on the composite key, so a Kick channel and a Twitch channel of
    // the same name are never mistaken for each other. Callers pass the row's
    // provider; absent means Twitch, as everywhere else.
    const isInMultiNook = useCallback((login: string, provider: ProviderId = 'twitch') => {
        const key = makeKey(provider, login);
        return multiNookSlots.some((s) => makeKey(s.provider ?? 'twitch', s.channelLogin) === key);
    }, [multiNookSlots]);

    // Use store state directly
    const activeTab = homeActiveTab;
    const selectedCategory = homeSelectedCategory;

    // Wrapper functions to update store state
    const setActiveTab = (tab: HomeTab) => setHomeActiveTab(tab);
    const setSelectedCategory = (category: TwitchCategory | null) => setHomeSelectedCategory(category);

    const [searchQuery, setSearchQuery] = useState('');
    const [searchResults, setSearchResults] = useState<TwitchStream[]>([]);
    const [categorySearchResults, setCategorySearchResults] = useState<TwitchCategory[]>([]);
    const [searchMode, setSearchMode] = useState<'streamers' | 'categories'>('streamers');
    const [isSearching, setIsSearching] = useState(false);
    // Recent searches shown under the search bar when it's focused and empty.
    const [recentSearches, setRecentSearches] = useState<RecentSearch[]>([]);
    const [historyOpen, setHistoryOpen] = useState(false);
    // Search history is scoped to the view a search launches from, so Following,
    // Discover, and Categories each keep their own separate list.
    const searchScope = useMemo(() => {
        const origin = activeTab === 'search' ? searchReturnTab : activeTab;
        switch (origin) {
            case 'following':
                return 'following';
            case 'browse':
            case 'category':
                return 'categories';
            default:
                return 'discover';
        }
    }, [activeTab, searchReturnTab]);
    const scopeLabel =
        searchScope === 'following' ? 'Following' : searchScope === 'categories' ? 'Categories' : 'Discover';
    // Reload the visible list when the scope changes (switching tabs) and each
    // time the dropdown opens, so an external clear (e.g. the command palette's
    // "Clear search history") is reflected without needing a tab switch.
    useEffect(() => {
        setRecentSearches(loadRecentSearches(searchScope));
    }, [searchScope, historyOpen]);

    // Guard against landing on an orphaned "search" tab. Search query/results are
    // local state, so they reset whenever Home remounts (e.g. after exiting a
    // stream that was opened from a search result) while the store keeps the tab
    // pinned to 'search'. That combination renders the empty "No channels found
    // for ''" page, which should never be reachable. Detect it and bounce back to
    // wherever the search was launched from. Runs before paint so there's no flash.
    // It can only fire in this orphaned case: clearing the search box (X/Escape)
    // already resets the tab, and a real zero-result search leaves searchQuery set.
    useLayoutEffect(() => {
        if (
            activeTab === 'search' &&
            !isSearching &&
            !searchQuery.trim() &&
            searchResults.length === 0 &&
            categorySearchResults.length === 0
        ) {
            const fallbackTab: HomeTab = isAuthenticated ? 'following' : 'recommended';
            const target =
                searchReturnTab === 'search' || (searchReturnTab === 'category' && !selectedCategory)
                    ? fallbackTab
                    : searchReturnTab;
            setActiveTab(target);
        }
    // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [activeTab, isSearching, searchQuery, searchResults.length, categorySearchResults.length]);
    const topGames = cachedTopGames;
    const [isLoadingGames, setIsLoadingGames] = useState(false);
    const gamesCursor = cachedGamesCursor;
    const hasMoreGames = cachedHasMoreGames;
    const [isLoadingMoreGames, setIsLoadingMoreGames] = useState(false);
    // --- Platform filter ----------------------------------------------------
    // Which platform the grid is showing. 'all' is Twitch's own tabs exactly as
    // before; picking a platform swaps the grid to that platform's directory (or
    // its followed channels on the Following tab). Only platforms whose watch
    // adapter has shipped appear, so this row is invisible until one has.
    // Follows the app-wide platform context set by the sidebar's switcher, so
    // Home and the sidebar can never disagree about which platform you're on.
    const providerFilter = useAppStore((s) => s.activePlatform);
    const [providerStreams, setProviderStreams] = useState<TwitchStream[]>([]);
    const [isLoadingProvider, setIsLoadingProvider] = useState(false);
    const [providerError, setProviderError] = useState<string | null>(null);
    const providerFollowsLive = useFollowsStore((s) => s.liveByKey);
    // Live rows for favourites the backend sweeps because they're followed
    // nowhere. Merged with the two follow sources below, not used on its own.
    const favoritesLive = useFavoritesStore((s) => s.liveByKey);
    const providerFollows = useFollowsStore((s) => s.follows);
    // Scoped to one non-Twitch platform: the grid is entirely that platform's.
    const isProviderView = providerFilter !== 'all' && providerFilter !== 'twitch';
    // Connection state, read from the account store rather than inferred from
    // "has this platform any follows". Those are different questions, and
    // answering the first with the second told a connected user with an empty
    // list to go and connect.
    // Field selectors, not a whole-store subscription: Home is expensive to
    // re-render and must not do it whenever an unrelated account field changes.
    // Twitch and the unified view answer `true` — they have their own gate.
    const providerConnected = usePlatformAccountStore((s) =>
        providerFilter === 'kick'
            ? s.kick.connected
            : providerFilter === 'youtube'
              ? s.youtube.connected
              : true,
    );
    const providerConnecting = usePlatformAccountStore((s) =>
        providerFilter === 'kick'
            ? s.kick.busy
            : providerFilter === 'youtube'
              ? s.youtube.busy
              : false,
    );
    const connectPlatformAccount = usePlatformAccountStore((s) => s.connect);
    // The unified view. Twitch's own surfaces still render (they're the richest),
    // and every other platform's live rows are folded in alongside them, ranked
    // together by viewers. Anything with no cross-platform equivalent — drops,
    // hype trains, watch streaks — simply stays absent from the provider rows.
    const isUnifiedView = providerFilter === 'all' && WATCHABLE_PROVIDERS.length > 1;
    // Categories for the selected platform, and which one is drilled into. The
    // Categories tab shows tiles like Twitch's; picking one lists its streams.
    const [providerCategories, setProviderCategories] = useState<ProviderCategory[]>([]);
    const [providerCategory, setProviderCategory] = useState<ProviderCategory | null>(null);

    const [categoryStreams, setCategoryStreams] = useState<TwitchStream[]>([]);
    const [categoryStreamsCursor, setCategoryStreamsCursor] = useState<string | null>(null);
    const [hasMoreCategoryStreams, setHasMoreCategoryStreams] = useState(true);
    const [isLoadingMoreCategoryStreams, setIsLoadingMoreCategoryStreams] = useState(false);
    const [isLoadingCategoryStreams, setIsLoadingCategoryStreams] = useState(false);

    // Tag-filtered live streams: fetched server-side (GQL) so a tag filter
    // returns every matching stream by viewer count, like Twitch's directory —
    // not just whatever happens to be on the loaded Helix pages. Active only
    // while one or more tags are selected; otherwise the Helix `categoryStreams`
    // above is shown.
    const [tagStreams, setTagStreams] = useState<TwitchStream[]>([]);
    const [tagStreamsCursor, setTagStreamsCursor] = useState<string | null>(null);
    const [hasMoreTagStreams, setHasMoreTagStreams] = useState(false);
    const [isLoadingTagStreams, setIsLoadingTagStreams] = useState(false);
    const [isLoadingMoreTagStreams, setIsLoadingMoreTagStreams] = useState(false);

    // Live-tab tag filter. `Draft` is the text in the search box (used to
    // autocomplete tag suggestions); committing a tag pushes it into
    // `selectedCategoryTags`, which drives the server-side tag fetch.
    const [categoryLiveDraft, setCategoryLiveDraft] = useState('');
    const [selectedCategoryTags, setSelectedCategoryTags] = useState<string[]>([]);

    // Category Tabs State
    const [categoryActiveTab, setCategoryActiveTabLocal] = useState<'live' | 'clips' | 'videos'>(homeCategoryTab);
    // Wrapper that syncs local and store state
    const setCategoryActiveTab = useCallback((tab: 'live' | 'clips' | 'videos') => {
        setCategoryActiveTabLocal(tab);
        setHomeCategoryTab(tab);
    }, [setHomeCategoryTab]);
    // Sync from store → local when navigating back from a clip/VOD
    useEffect(() => {
        setCategoryActiveTabLocal(homeCategoryTab);
    }, [homeCategoryTab]);
    const [categoryClips, setCategoryClips] = useState<TwitchClip[]>([]);
    const [categoryClipsCursor, setCategoryClipsCursor] = useState<string | null>(null);
    const [hasMoreCategoryClips, setHasMoreCategoryClips] = useState(true);
    const [isLoadingClips, setIsLoadingClips] = useState(false);
    const [isLoadingMoreClips, setIsLoadingMoreClips] = useState(false);
    
    const [categoryVideos, setCategoryVideos] = useState<TwitchVideo[]>([]);
    const [categoryVideosCursor, setCategoryVideosCursor] = useState<string | null>(null);
    const [hasMoreCategoryVideos, setHasMoreCategoryVideos] = useState(true);
    const [isLoadingVideos, setIsLoadingVideos] = useState(false);
    const [isLoadingMoreVideos, setIsLoadingMoreVideos] = useState(false);

    const [categoryDetails, setCategoryDetails] = useState<CategoryInfo | null>(null);
    const [isLoadingCategoryDetails, setIsLoadingCategoryDetails] = useState(false);
    const [isDescriptionExpanded, setIsDescriptionExpanded] = useState(false);
    const [isDescriptionClamped, setIsDescriptionClamped] = useState(true);
    const [animatingHearts, setAnimatingHearts] = useState<Set<string>>(new Set());
    const [isSearchExpanded, setIsSearchExpanded] = useState(false);
    const searchInputRef = useRef<HTMLInputElement>(null);
    // The recent-searches dropdown is portaled to <body> because the top nav frame
    // clips overflow — an in-flow absolute dropdown gets cut off and reads as
    // invisible. We anchor it to the search bar's rect instead.
    const searchBarRef = useRef<HTMLDivElement>(null);
    const [historyRect, setHistoryRect] = useState<{ top: number; left: number; width: number } | null>(null);

    const isMountedRef = useRef(true);
    useEffect(() => {
        // Re-arm on (re)setup, not only clear on cleanup. React StrictMode runs
        // setup -> cleanup -> setup on mount in dev; without setting true here the
        // cleanup's `false` sticks for the component's whole life, silently
        // dropping every async .then/.finally below (category stream + detail
        // loads then spin forever because their loading flag is never cleared).
        isMountedRef.current = true;
        return () => { isMountedRef.current = false; };
    }, []);

    // A view that needs drop indicators before Rust has filled them asks for
    // a refresh (15 s floor over there); the result lands through the snapshot.
    const refreshDrops = useCallback(() => {
        void invoke('refresh_home_section', { section: 'drops' }).catch(() => {});
    }, []);
    // Drops-enabled categories (by game_id) and by lower-cased game name, both
    // derived from the campaign list Rust keeps in the Home snapshot.
    const dropsCampaigns = useAppStore((s) => s.dropsCampaigns);
    const dropsGameIds = useMemo(() => {
        const map = new Map<string, DropCampaign>();
        for (const campaign of dropsCampaigns) {
            if (campaign.game_id) map.set(campaign.game_id, campaign);
        }
        return map;
    }, [dropsCampaigns]);
    const dropsGameNames = useMemo(() => {
        const map = new Map<string, DropCampaign>();
        for (const campaign of dropsCampaigns) {
            if (campaign.game_name) map.set(campaign.game_name.toLowerCase(), campaign);
        }
        return map;
    }, [dropsCampaigns]);

    const scrollContainerRef = useRef<HTMLDivElement>(null);
    const heroSentinelRef = useRef<HTMLDivElement>(null);
    const [isScrolledPastHero, setIsScrolledPastHero] = useState(false);

    useEffect(() => {
        const container = scrollContainerRef.current;
        const sentinel = heroSentinelRef.current;
        if (!container || !sentinel) return;

        const observer = new IntersectionObserver(
            ([entry]) => setIsScrolledPastHero(!entry.isIntersecting),
            { root: container, rootMargin: '0px', threshold: 0 }
        );
        observer.observe(sentinel);
        return () => observer.disconnect();
    }, []);

    const loadingRef = useRef(false);
    
    // Debounce ref for Hype Train status refresh
    const hypeTrainRefreshTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

    // Track if the Home component has initialized (to avoid showing LoadingWidget on user-initiated login)
    const [hasInitialized, setHasInitialized] = useState(false);

    useEffect(() => {
        // Mark as initialized immediately so we render cached store data if available
        setHasInitialized(true);

        // Rust owns the Home data (src-tauri/src/services/home_snapshot.rs).
        // Mounting hydrates the store from the current snapshot (no network on
        // the critical path) and tells Rust a Home is on screen, which refreshes
        // any stale section right away and runs the recommended poll while we
        // are up. The old 300 ms deferred refetch is gone: nothing here fetches.
        useAppStore.setState((state) => ({ homeOpenCount: state.homeOpenCount + 1 }));
        void ensureHomeSnapshotSync();
        void invoke('set_home_mounted', { mounted: true }).catch(() => {});

        return () => {
            void invoke('set_home_mounted', { mounted: false }).catch(() => {});
        };
    }, []);

    // Save the grid's scroll offset for the next mount. A layout-effect cleanup
    // runs before the node leaves the DOM (a passive cleanup would read 0).
    useLayoutEffect(() => {
        const container = scrollContainerRef.current;
        return () => {
            if (container) useAppStore.setState({ homeScrollTop: container.scrollTop });
        };
    }, []);

    // Reopen where the user left off: the grid restores its scroll offset once
    // its data is on screen. The stagger below runs on the first mount only.
    const homeOpenCount = useAppStore((s) => s.homeOpenCount);
    const isReopen = homeOpenCount > 1;
    useLayoutEffect(() => {
        if (!isReopen) return;
        const container = scrollContainerRef.current;
        const top = useAppStore.getState().homeScrollTop;
        if (container && top > 0) container.scrollTop = top;
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [followedStreams.length, recommendedStreams.length]);

    // Scroll-Collapse Header Observer has been completely replaced by native framer-motion useScroll progressive tracking!    // Auto-select the appropriate tab based on auth status on initial mount only
    // This effect should NOT run when user clicks tabs - remove homeActiveTab from deps
    useEffect(() => {
        // Don't override if user has navigated to a specific tab
        // Only auto-select on initial mount or when auth status truly changes
        if (homeActiveTab === 'category' || homeActiveTab === 'search' || homeActiveTab === 'browse') {
            return;
        }
        if (isAuthenticated && followedStreams.length > 0) {
            setActiveTab('following');
        } else if (!isAuthenticated || followedStreams.length === 0) {
            setActiveTab('recommended');
        }
        // Note: homeActiveTab intentionally not in deps to prevent feedback loop
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [isAuthenticated, followedStreams.length]);

    // Focus search input when expanded
    useEffect(() => {
        if (isSearchExpanded && searchInputRef.current) {
            searchInputRef.current.focus();
        }
    }, [isSearchExpanded]);

    // Keep the portaled recent-searches dropdown anchored under the search bar.
    // The top nav doesn't scroll, so the rect only shifts on window resize.
    useEffect(() => {
        if (!(isSearchExpanded && historyOpen)) {
            setHistoryRect(null);
            return;
        }
        const update = () => {
            const el = searchBarRef.current;
            if (!el) return;
            const r = el.getBoundingClientRect();
            setHistoryRect({ top: r.bottom + 8, left: r.left, width: r.width });
        };
        update();
        window.addEventListener('resize', update);
        return () => window.removeEventListener('resize', update);
    }, [isSearchExpanded, historyOpen]);

    const loadTopGames = async (background = false) => {
        if (!background) {
            setIsLoadingGames(true);
        }
        try {
            const [games, cursor] = await invoke('get_top_games_paginated', {
                cursor: null,
                limit: 40
            }) as [TwitchCategory[], string | null];
            
            if (background && cachedTopGames.length > 40) {
                // Preserve scroll-loaded pages: replace only page 1, keep pages 2+
                const preservedPages = cachedTopGames.slice(40);
                setCachedTopGames([...games, ...preservedPages], cachedGamesCursor, cachedHasMoreGames);
            } else {
                setCachedTopGames(games, cursor, !!cursor);
            }
        } catch (e) {
            Logger.error('Failed to load top games:', e);
            if (!background) {
                setCachedTopGames([], null, false);
            }
        } finally {
            if (!background) {
                setIsLoadingGames(false);
            }
        }
    };

    const loadMoreTopGames = useCallback(async () => {
        if (!hasMoreGames || isLoadingMoreGames || !gamesCursor) return;

        setIsLoadingMoreGames(true);
        try {
            const [games, cursor] = await invoke('get_top_games_paginated', {
                cursor: gamesCursor,
                limit: 40
            }) as [TwitchCategory[], string | null];
            appendCachedTopGames(games, cursor, !!cursor);
        } catch (e) {
            Logger.error('Failed to load more top games:', e);
        } finally {
            setIsLoadingMoreGames(false);
        }
    }, [hasMoreGames, isLoadingMoreGames, gamesCursor, appendCachedTopGames]);

    // Load active drops campaigns and build maps for both game_id and game_name lookup
    // State for automation animation and tracking actively automation campaigns
    const [activeAutomationIds, setActiveAutomationIds] = useState<Set<string>>(new Set());
    const [flyingDroplet, setFlyingDroplet] = useState<{ visible: boolean; x: number; y: number } | null>(null);

    // Create a map from campaign name to campaign ID for reverse lookup
    const campaignNameToIdRef = useRef<Map<string, string>>(new Map());

    // Highlight the campaign a running automation is farming, from the
    // bridge-cached progress and the Rust-kept campaign list.
    const liveDropProgressForHome = useAppStore((s) => s.liveDropProgress);
    useEffect(() => {
        if (!liveDropProgressForHome?.active || dropsCampaigns.length === 0) return;
        const progressGameName = liveDropProgressForHome.current_drop?.game_name?.toLowerCase() ||
            liveDropProgressForHome.current_channel?.game_name?.toLowerCase();
        if (!progressGameName) return;
        const match = dropsCampaigns.find((c) => c.game_name?.toLowerCase() === progressGameName);
        if (match) setActiveAutomationIds(prev => (prev.has(match.id) ? prev : new Set(prev).add(match.id)));
    }, [liveDropProgressForHome, dropsCampaigns]);

    // Hype trains are polled in Rust for followed + recommended; the category
    // grid and search results are only known here, so hand their ids over
    // (debounced) and Rust folds them into the same poll.
    useEffect(() => {
        const ids = new Set<string>();
        categoryStreams.forEach(s => ids.add(s.user_id));
        searchResults.forEach(s => ids.add(s.user_id));
        if (hypeTrainRefreshTimeoutRef.current) {
            clearTimeout(hypeTrainRefreshTimeoutRef.current);
        }
        hypeTrainRefreshTimeoutRef.current = setTimeout(() => {
            void invoke('set_home_extra_channels', { channelIds: Array.from(ids) }).catch(() => {});
        }, 2000);
        return () => {
            if (hypeTrainRefreshTimeoutRef.current) {
                clearTimeout(hypeTrainRefreshTimeoutRef.current);
            }
        };
    }, [categoryStreams, searchResults]);

    // Sync automation status with backend. Real-time updates arrive via the
    // 'automation-status-changed' event listener below; the periodic call is a
    // stale-protection net that runs at 60-min cadence (aligned with the
    // TitleBar + ChatWidget backup polls) and only fires when the window is
    // visible.
    const syncAutomationStatus = useCallback(async () => {
        try {
            const dropProgress = useAppStore.getState().liveDropProgress;

            if (dropProgress?.active) {
                // Find campaign ID by matching game_name from current_drop or current_channel
                const progressGameName = dropProgress.current_drop?.game_name?.toLowerCase() ||
                    dropProgress.current_channel?.game_name?.toLowerCase();

                if (progressGameName) {
                    let foundCampaignId: string | null = null;
                    dropsGameNames.forEach((campaign, gameName) => {
                        if (gameName === progressGameName) {
                            foundCampaignId = campaign.id;
                        }
                    });

                    if (foundCampaignId) {
                        setActiveAutomationIds(prev => {
                            if (prev.size === 1 && prev.has(foundCampaignId!)) {
                                return prev;
                            }
                            return new Set([foundCampaignId!]);
                        });
                    }
                }
            } else {
                setActiveAutomationIds(prev => {
                    if (prev.size > 0) {
                        return new Set<string>();
                    }
                    return prev;
                });
            }
        } catch {
            // Silently fail - might not be authenticated or backend not ready
        }
    }, [dropsGameNames]);

    useEffect(() => {
        syncAutomationStatus();

        let unlisten: (() => void) | null = null;
        let isMounted = true;
        const setupListener = async () => {
            try {
                const { listen } = await import('@tauri-apps/api/event');
                const unlistenFn = await listen('drop-progress', syncAutomationStatus);
                if (isMounted) {
                    unlisten = unlistenFn;
                } else {
                    unlistenFn();
                }
            } catch {
                // Event listener not available
            }
        };
        setupListener();

        return () => {
            isMounted = false;
            if (unlisten) unlisten();
        };
    }, [syncAutomationStatus]);

    useVisibleInterval(syncAutomationStatus, 60 * 60 * 1000);

    // Update campaign name-to-ID map when drops data loads
    useEffect(() => {
        const nameToId = new Map<string, string>();
        dropsGameIds.forEach((campaign) => {
            nameToId.set(campaign.name, campaign.id);
        });
        campaignNameToIdRef.current = nameToId;
    }, [dropsGameIds]);

    // Handler to toggle automation drops for a category (start or stop)
    const handleToggleAutomation = async (e: React.MouseEvent, campaign: DropCampaign) => {
        e.stopPropagation(); // Don't trigger category click

        const isCurrentlyAutomating = activeAutomationIds.has(campaign.id);

        // Automation is plugin-powered; this control only renders when a plugin
        // provides automation, so route start/stop to it.
        if (!useAppStore.getState().externalDropsProvider) return;

        if (isCurrentlyAutomating) {
            // Stop automation
            try {
                await invoke('plugins_invoke_action', { action: 'drops.stop', args: {} });
                Logger.debug(`[Home] Stopped automation drops for ${campaign.name}`);
                setActiveAutomationIds(new Set()); // Clear all automation IDs
                useAppStore.getState().addToast(`Stopped automation drops for ${campaign.game_name}`, 'info');
            } catch (error) {
                Logger.error('Failed to stop automation:', error);
                useAppStore.getState().addToast('Failed to stop automation drops', 'error');
            }
        } else {
            // Start automation
            // Get button position for flying animation
            const button = e.currentTarget as HTMLElement;
            const rect = button.getBoundingClientRect();
            const centerX = rect.left + rect.width / 2;
            const centerY = rect.top + rect.height / 2;

            try {
                await invoke('plugins_invoke_action', { action: 'drops.run', args: { campaign_id: campaign.id } });
                Logger.debug(`[Home] Started automation drops for ${campaign.name}`);

                // Add to active automation set
                setActiveAutomationIds(new Set([campaign.id]));

                // Start flying droplet animation
                setFlyingDroplet({ visible: true, x: centerX, y: centerY });

                // Clear flying animation after it completes
                setTimeout(() => setFlyingDroplet(null), 1000);

                useAppStore.getState().addToast(`Started automation drops for ${campaign.game_name}`, 'success');
            } catch (error) {
                Logger.error('Failed to start automation:', error);
                useAppStore.getState().addToast('Failed to start automation drops', 'error');
            }
        }
    };

    const CATEGORY_CACHE_TTL = 60_000; // 60 seconds

    const handleBrowseClick = () => {
        setActiveTab('browse');
        setIsSearchExpanded(false);
        
        const isCacheStale = Date.now() - cachedTopGamesTimestamp > CATEGORY_CACHE_TTL;
        
        if (topGames.length === 0) {
            loadTopGames(false);
        } else if (isCacheStale) {
            loadTopGames(true);
        }
        
        // Also load drops data
        if (dropsGameIds.size === 0) {
            refreshDrops();
        }
    };

    const handleCategoryClick = async (category: TwitchCategory) => {
        setSelectedCategory(category);
        setActiveTab('category');
        setIsLoadingCategoryStreams(true);
        setCategoryStreams([]);
        setIsLoadingCategoryDetails(true);
        setCategoryDetails(null);
        setIsDescriptionExpanded(false);
        setCategoryActiveTab('live');
        setCategoryClips([]);
        setCategoryClipsCursor(null);
        setHasMoreCategoryClips(true);
        setCategoryVideos([]);
        setCategoryVideosCursor(null);
        setHasMoreCategoryVideos(true);

        invoke('get_streams_by_game', { gameId: category.id, cursor: null, limit: 40 })
            .then(res => {
                if (!isMountedRef.current) return;
                const [streams, cursor] = res as [TwitchStream[], string | null];
                setCategoryStreams(streams);
                setCategoryStreamsCursor(cursor);
                setHasMoreCategoryStreams(!!cursor && streams.length > 0);
            })
            .catch(e => {
                if (!isMountedRef.current) return;
                Logger.error('Failed to load category streams:', e);
                setCategoryStreams([]);
                setCategoryStreamsCursor(null);
                setHasMoreCategoryStreams(false);
            })
            .finally(() => { if (isMountedRef.current) setIsLoadingCategoryStreams(false); });

        invoke('get_category_info', { gameName: category.name })
            .then(details => { if (isMountedRef.current) setCategoryDetails(details as CategoryInfo | null); })
            .catch(e => { if (isMountedRef.current) Logger.error('Failed to load category details:', e); })
            .finally(() => { if (isMountedRef.current) setIsLoadingCategoryDetails(false); });
    };

    // Load streams by game name when navigating from badge overlay (category has no ID)
    const loadCategoryStreamsByName = async (gameName: string) => {
        setIsLoadingCategoryStreams(true);
        setCategoryStreams([]);
        setIsLoadingCategoryDetails(true);
        setCategoryDetails(null);
        setIsDescriptionExpanded(false);
        setCategoryActiveTab('live');
        setCategoryClips([]);
        setCategoryClipsCursor(null);
        setHasMoreCategoryClips(true);
        setCategoryVideos([]);
        setCategoryVideosCursor(null);
        setHasMoreCategoryVideos(true);

        invoke('get_streams_by_game_name', { gameName: gameName, excludeUserLogin: null, cursor: null, limit: 40 })
            .then(res => {
                if (!isMountedRef.current) return;
                const [streams, cursor] = res as [TwitchStream[], string | null];
                setCategoryStreams(streams);
                setCategoryStreamsCursor(cursor);
                setHasMoreCategoryStreams(!!cursor && streams.length > 0);
            })
            .catch(e => {
                if (!isMountedRef.current) return;
                Logger.error('Failed to load category streams by name:', e);
                setCategoryStreams([]);
                setCategoryStreamsCursor(null);
                setHasMoreCategoryStreams(false);
            })
            .finally(() => { if (isMountedRef.current) setIsLoadingCategoryStreams(false); });

        invoke('get_category_info', { gameName: gameName })
            .then(details => { if (isMountedRef.current) setCategoryDetails(details as CategoryInfo | null); })
            .catch(e => { if (isMountedRef.current) Logger.error('Failed to load category details:', e); })
            .finally(() => { if (isMountedRef.current) setIsLoadingCategoryDetails(false); });
    };

    // Effect to handle category view re-mount or navigation from badge overlay
    useEffect(() => {
        if (activeTab === 'category' && selectedCategory) {
            if (selectedCategory.id) {
                // Normal category — re-fetch if streams are empty (e.g., after remount from watching a stream)
                if (categoryStreams.length === 0 && !isLoadingCategoryStreams) {
                    // Don't call handleCategoryClick here — it resets categoryActiveTab to 'live'.
                    // When navigating back from a clip/VOD, we need to preserve the sub-tab.
                    // Just reload the stream data without touching tab state.
                    setIsLoadingCategoryStreams(true);
                    invoke('get_streams_by_game', { gameId: selectedCategory.id, cursor: null, limit: 40 })
                        .then(res => {
                            if (!isMountedRef.current) return;
                            const [streams, cursor] = res as [TwitchStream[], string | null];
                            setCategoryStreams(streams);
                            setCategoryStreamsCursor(cursor);
                            setHasMoreCategoryStreams(!!cursor && streams.length > 0);
                        })
                        .catch(e => {
                            if (!isMountedRef.current) return;
                            Logger.error('Failed to load category streams:', e);
                            setCategoryStreams([]);
                            setCategoryStreamsCursor(null);
                            setHasMoreCategoryStreams(false);
                        })
                        .finally(() => { if (isMountedRef.current) setIsLoadingCategoryStreams(false); });
                }
            } else if (selectedCategory.name) {
                // Badge overlay navigation — category has no ID, load by name
                loadCategoryStreamsByName(selectedCategory.name);
            }
        }
    // categoryStreams.length intentionally excluded to avoid re-fetch loops after legitimate empty results
    // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [activeTab, selectedCategory]);

    // Effect to auto-load categories when browse tab is active but topGames were lost (e.g., remount)
    useEffect(() => {
        const isCacheStale = Date.now() - cachedTopGamesTimestamp > CATEGORY_CACHE_TTL;
        if (activeTab === 'browse' && topGames.length === 0 && !isLoadingGames) {
            loadTopGames(false);
        } else if (activeTab === 'browse' && isCacheStale && !isLoadingGames) {
            loadTopGames(true);
        }
        if (activeTab === 'browse' && dropsGameIds.size === 0) {
            refreshDrops();
        }
    // topGames.length intentionally excluded to avoid re-fetch loops
    // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [activeTab]);

    const handleBackToBrowse = () => {
        setActiveTab('browse');
        setSelectedCategory(null);
        setCategoryStreams([]);
        setCategoryStreamsCursor(null);
        setHasMoreCategoryStreams(true);
        
        const isCacheStale = Date.now() - cachedTopGamesTimestamp > CATEGORY_CACHE_TTL;
        
        // Re-fetch categories if lost on remount
        if (topGames.length === 0) {
            loadTopGames(false);
        } else if (isCacheStale) {
            loadTopGames(true);
        }
        
        if (dropsGameIds.size === 0) {
            refreshDrops();
        }
    };

    const loadMoreCategoryStreams = useCallback(async () => {
        if (!selectedCategory || !hasMoreCategoryStreams || isLoadingMoreCategoryStreams) return;
        setIsLoadingMoreCategoryStreams(true);
        try {
            if (selectedCategory.id) {
                const res = await invoke('get_streams_by_game', { 
                    gameId: selectedCategory.id, 
                    cursor: categoryStreamsCursor, 
                    limit: 40 
                }) as [TwitchStream[], string | null];
                const [newStreams, newCursor] = res;
                if (newStreams.length > 0) {
                    setCategoryStreams(prev => {
                        // Twitch's category stream list is ordered by viewer count and
                        // shifts between page fetches, so a stream near a page boundary
                        // can come back on the next page. Drop already-present streams
                        // to avoid duplicate React keys (and duplicate cards).
                        const seen = new Set(prev.map(s => s.id));
                        return [...prev, ...newStreams.filter(s => !seen.has(s.id))];
                    });
                    setCategoryStreamsCursor(newCursor);
                    setHasMoreCategoryStreams(!!newCursor);
                } else {
                    setHasMoreCategoryStreams(false);
                }
            } else if (selectedCategory.name) {
                const res = await invoke('get_streams_by_game_name', { 
                    gameName: selectedCategory.name, 
                    excludeUserLogin: null, 
                    cursor: categoryStreamsCursor, 
                    limit: 40 
                }) as [TwitchStream[], string | null];
                const [newStreams, newCursor] = res;
                if (newStreams.length > 0) {
                    setCategoryStreams(prev => {
                        // Twitch's category stream list is ordered by viewer count and
                        // shifts between page fetches, so a stream near a page boundary
                        // can come back on the next page. Drop already-present streams
                        // to avoid duplicate React keys (and duplicate cards).
                        const seen = new Set(prev.map(s => s.id));
                        return [...prev, ...newStreams.filter(s => !seen.has(s.id))];
                    });
                    setCategoryStreamsCursor(newCursor);
                    setHasMoreCategoryStreams(!!newCursor);
                } else {
                    setHasMoreCategoryStreams(false);
                }
            }
        } catch (e) {
            Logger.error('Failed to load more category streams:', e);
            setHasMoreCategoryStreams(false);
        } finally {
            setIsLoadingMoreCategoryStreams(false);
        }
    }, [selectedCategory, hasMoreCategoryStreams, isLoadingMoreCategoryStreams, categoryStreamsCursor]);

    const loadCategoryClips = async () => {
        if (!selectedCategory?.id) return;
        setIsLoadingClips(true);
        setCategoryClips([]);
        setCategoryClipsCursor(null);
        setHasMoreCategoryClips(true);
        try {
            const res = await invoke('get_clips_by_game', { gameId: selectedCategory.id, limit: 40, cursor: null, period: clipsPeriod }) as [TwitchClip[], string | null];
            setCategoryClips(res[0]);
            setCategoryClipsCursor(res[1]);
            // Clips often don't have cursors if they reach the end in the first page, Twitch pagination is finicky
            setHasMoreCategoryClips(!!res[1] && res[0].length >= 40);
        } catch (e) {
            Logger.error('Failed to load category clips:', e);
            setHasMoreCategoryClips(false);
        } finally {
            setIsLoadingClips(false);
        }
    };

    const loadMoreCategoryClips = useCallback(async () => {
        if (!selectedCategory?.id || !hasMoreCategoryClips || isLoadingMoreClips) return;
        setIsLoadingMoreClips(true);
        try {
            const res = await invoke('get_clips_by_game', { gameId: selectedCategory.id, limit: 40, cursor: categoryClipsCursor, period: clipsPeriod }) as [TwitchClip[], string | null];
            if (res[0].length > 0) {
                setCategoryClips(prev => {
                    const seen = new Set(prev.map(c => c.id));
                    return [...prev, ...res[0].filter(c => !seen.has(c.id))];
                });
                setCategoryClipsCursor(res[1]);
                setHasMoreCategoryClips(!!res[1] && res[0].length >= 40);
            } else {
                setHasMoreCategoryClips(false);
            }
        } catch (e) {
            Logger.error('Failed to load more category clips:', e);
            setHasMoreCategoryClips(false);
        } finally {
            setIsLoadingMoreClips(false);
        }
    }, [selectedCategory, hasMoreCategoryClips, isLoadingMoreClips, categoryClipsCursor, clipsPeriod]);

    const loadCategoryVideos = async () => {
        if (!selectedCategory?.id) return;
        setIsLoadingVideos(true);
        setCategoryVideos([]);
        setCategoryVideosCursor(null);
        setHasMoreCategoryVideos(true);
        try {
            const res = await invoke('get_videos_by_game', { gameId: selectedCategory.id, sort: videosSort, period: videosPeriod, limit: 40, cursor: null }) as [TwitchVideo[], string | null];
            setCategoryVideos(res[0]);
            setCategoryVideosCursor(res[1]);
            setHasMoreCategoryVideos(!!res[1] && res[0].length >= 40);
        } catch (e) {
            Logger.error('Failed to load category videos:', e);
            setHasMoreCategoryVideos(false);
        } finally {
            setIsLoadingVideos(false);
        }
    };

    const loadMoreCategoryVideos = useCallback(async () => {
        if (!selectedCategory?.id || !hasMoreCategoryVideos || isLoadingMoreVideos) return;
        setIsLoadingMoreVideos(true);
        try {
            const res = await invoke('get_videos_by_game', { gameId: selectedCategory.id, sort: videosSort, period: videosPeriod, limit: 40, cursor: categoryVideosCursor }) as [TwitchVideo[], string | null];
            if (res[0].length > 0) {
                setCategoryVideos(prev => {
                    const seen = new Set(prev.map(v => v.id));
                    return [...prev, ...res[0].filter(v => !seen.has(v.id))];
                });
                setCategoryVideosCursor(res[1]);
                setHasMoreCategoryVideos(!!res[1] && res[0].length >= 40);
            } else {
                setHasMoreCategoryVideos(false);
            }
        } catch (e) {
            Logger.error('Failed to load more category videos:', e);
            setHasMoreCategoryVideos(false);
        } finally {
            setIsLoadingMoreVideos(false);
        }
    }, [selectedCategory, hasMoreCategoryVideos, isLoadingMoreVideos, categoryVideosCursor, videosSort, videosPeriod]);

    // Effect to trigger fetching clips/videos on tab change
    useEffect(() => {
        if (activeTab === 'category' && selectedCategory?.id) {
            if (categoryActiveTab === 'clips' && categoryClips.length === 0 && !isLoadingClips) {
                loadCategoryClips();
            } else if (categoryActiveTab === 'videos' && categoryVideos.length === 0 && !isLoadingVideos) {
                loadCategoryVideos();
            }
        }
    // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [activeTab, categoryActiveTab, selectedCategory]);

    // Clear the live tag/text filters when moving to a different category.
    useEffect(() => {
        setCategoryLiveDraft('');
        setSelectedCategoryTags([]);
        setTagStreams([]);
        setTagStreamsCursor(null);
        setHasMoreTagStreams(false);
    }, [selectedCategory?.id]);

    // Fetch tag-filtered streams server-side whenever the selected tags change.
    const selectedTagsKey = selectedCategoryTags.map(t => t.toLowerCase()).sort().join('');
    useEffect(() => {
        const gameName = selectedCategory?.name;
        if (!gameName || selectedCategoryTags.length === 0) {
            setTagStreams([]);
            setTagStreamsCursor(null);
            setHasMoreTagStreams(false);
            return;
        }
        let cancelled = false;
        setIsLoadingTagStreams(true);
        invoke('get_streams_by_game_with_tags', { gameName, tags: selectedCategoryTags, cursor: null, limit: 40 })
            .then(res => {
                if (cancelled || !isMountedRef.current) return;
                const [streams, cursor] = res as [TwitchStream[], string | null];
                setTagStreams(streams);
                setTagStreamsCursor(cursor);
                setHasMoreTagStreams(!!cursor && streams.length > 0);
            })
            .catch(e => {
                if (cancelled || !isMountedRef.current) return;
                Logger.error('Failed to load tag-filtered streams:', e);
                setTagStreams([]);
                setTagStreamsCursor(null);
                setHasMoreTagStreams(false);
            })
            .finally(() => { if (!cancelled && isMountedRef.current) setIsLoadingTagStreams(false); });
        return () => { cancelled = true; };
    // selectedTagsKey captures the tag set; selectedCategory?.name keys the category.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [selectedTagsKey, selectedCategory?.name]);

    const loadMoreTagStreams = useCallback(async () => {
        const gameName = selectedCategory?.name;
        if (!gameName || selectedCategoryTags.length === 0) return;
        if (!hasMoreTagStreams || isLoadingMoreTagStreams || !tagStreamsCursor) return;
        setIsLoadingMoreTagStreams(true);
        try {
            const res = await invoke('get_streams_by_game_with_tags', {
                gameName,
                tags: selectedCategoryTags,
                cursor: tagStreamsCursor,
                limit: 40,
            }) as [TwitchStream[], string | null];
            if (!isMountedRef.current) return;
            const [streams, cursor] = res;
            setTagStreams(prev => {
                const seen = new Set(prev.map(s => s.user_id));
                return [...prev, ...streams.filter(s => !seen.has(s.user_id))];
            });
            setTagStreamsCursor(cursor);
            setHasMoreTagStreams(!!cursor && streams.length > 0);
        } catch (e) {
            if (isMountedRef.current) {
                Logger.error('Failed to load more tag-filtered streams:', e);
                setHasMoreTagStreams(false);
            }
        } finally {
            if (isMountedRef.current) setIsLoadingMoreTagStreams(false);
        }
    }, [selectedCategory?.name, selectedCategoryTags, hasMoreTagStreams, isLoadingMoreTagStreams, tagStreamsCursor]);

    // Refresh clips when period changes
    useEffect(() => {
        if (activeTab === 'category' && categoryActiveTab === 'clips' && selectedCategory?.id) {
            loadCategoryClips();
        }
    // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [clipsPeriod]);

    // Refresh videos when sort or period changes
    useEffect(() => {
        if (activeTab === 'category' && categoryActiveTab === 'videos' && selectedCategory?.id) {
            loadCategoryVideos();
        }
    // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [videosSort, videosPeriod]);

    // `opts` lets a recent-search entry replay itself: `query` overrides the box
    // text and `mode` forces streamer- or category-search regardless of the tab
    // it's launched from. Called with no args from the input (Enter), it keeps
    // the original tab-driven behavior.
    const handleSearch = async (opts?: { query?: string; mode?: SearchMode }) => {
        const q = (opts?.query ?? searchQuery).trim();
        if (!q) return;
        if (opts?.query !== undefined && opts.query !== searchQuery) {
            setSearchQuery(opts.query);
        }
        setHistoryOpen(false);

        // Remember where this search was launched from. Search query/results live
        // in local state, so they're wiped when Home unmounts during playback;
        // without this, exiting a stream opened from a result drops the user on an
        // empty "search" tab (the "No channels found for ''" blank page).
        if (activeTab !== 'search') {
            setSearchReturnTab(activeTab);
        }

        setIsSearching(true);
        let usedMode: SearchMode = 'streamers';
        try {
            const forceStreamers = opts?.mode === 'streamers';
            const forceCategories = opts?.mode === 'categories';
            if (!forceStreamers && (forceCategories || activeTab === 'browse' || (activeTab === 'search' && searchMode === 'categories'))) {
                usedMode = 'categories';
                setSearchMode('categories');
                setActiveTab('search');
                const results = await invoke('search_categories', { query: q, limit: 40 }) as TwitchCategory[];
                setCategorySearchResults(results);
                setSearchResults([]);
            } else if (!forceStreamers && ((activeTab === 'category' && selectedCategory) || (activeTab === 'search' && searchMode === 'streamers' && selectedCategory))) {
                usedMode = 'streamers';
                setSearchMode('streamers');
                setActiveTab('search');
                const results = await invoke('search_channels', { query: q }) as TwitchStream[];

                const filtered = results.filter(s =>
                    (selectedCategory.id && s.game_id === selectedCategory.id) ||
                    (selectedCategory.name && s.game_name?.toLowerCase() === selectedCategory.name.toLowerCase())
                );

                setSearchResults(filtered);
                setCategorySearchResults([]);
            } else if (isProviderView) {
                // Scoped to one platform: search THAT platform. Without this the
                // search tab fell through to rendering the whole directory, so a
                // name search returned everything that was live.
                usedMode = 'streamers';
                setSearchMode('streamers');
                setActiveTab('search');
                const page = await invoke<{ streams: TwitchStream[] }>('provider_search', {
                    provider: providerFilter,
                    query: q,
                });
                setSearchResults(page.streams ?? []);
                setCategorySearchResults([]);
            } else {
                usedMode = 'streamers';
                setSearchMode('streamers');
                setActiveTab('search');
                // Unified: Twitch plus every platform that advertises search, the
                // same way unified Discover merges directories. Each platform is
                // settled INDEPENDENTLY so one failing never empties the results.
                const others = isUnifiedView
                    ? WATCHABLE_PROVIDERS.filter((pv) => pv !== 'twitch' && PROVIDER_WATCH[pv].search)
                    : [];
                const [twitchResults, ...providerPages] = await Promise.all([
                    (invoke('search_channels', { query: q }) as Promise<TwitchStream[]>).catch(() => []),
                    ...others.map((pv) =>
                        invoke<{ streams: TwitchStream[] }>('provider_search', { provider: pv, query: q })
                            .then((page) => page.streams ?? [])
                            .catch(() => [] as TwitchStream[]),
                    ),
                ]);
                setSearchResults([
                    ...(twitchResults ?? []),
                    ...providerPages.flat(),
                ]);
                setCategorySearchResults([]);
            }
            setRecentSearches(addRecentSearch(searchScope, q, usedMode));
        } catch (e) {
            Logger.error('Search failed:', e);
            setSearchResults([]);
            setCategorySearchResults([]);
        } finally {
            setIsSearching(false);
        }
    };

    const handleSearchKeyPress = (e: React.KeyboardEvent) => {
        if (e.key === 'Enter') {
            handleSearch();
        } else if (e.key === 'Escape') {
            setIsSearchExpanded(false);
            setHistoryOpen(false);
            setSearchQuery('');
            setSearchResults([]);
            setCategorySearchResults([]);
            if (activeTab === 'search') {
                setActiveTab(isAuthenticated ? 'following' : 'recommended');
            }
        }
    };

    const getThumbnailUrl = (url: string) => {
        return url
            .replace('%{width}', '640').replace('%{height}', '360')
            .replace('{width}', '640').replace('{height}', '360');
    };

    const getGameBoxArt = (url: string) => {
        if (!url) return '';
        if (url.includes('{width}') && url.includes('{height}')) {
            return url.replace('{width}', '1200').replace('{height}', '1600');
        }
        return url.replace(/-\d+x\d+\.(jpg|jpeg|png)$/i, '-1200x1600.$1');
    };

    const handleStreamClick = (e: React.MouseEvent, stream: TwitchStream) => {
        const provider = streamProvider(stream);
        // Ctrl/Cmd+click adds the stream to multinook instead of switching to it.
        // The flying-card animation originates from the click point so it visually
        // matches the right-click context-menu "Add to MultiNook" action.
        if (e.ctrlKey || e.metaKey) {
            e.preventDefault();
            // No refusal here: addSlot owns the grid gate and reports its own
            // reason, so a second copy of the rule would only drift from it.
            usemultiNookStore.getState().triggerAddAnimation(e.clientX, e.clientY, stream.user_login, provider);
            usemultiNookStore.getState().addSlot(stream.user_login, provider);
            return;
        }
        // Track which category this stream was started from (if any)
        useAppStore.getState().setStreamOriginCategory(
            activeTab === 'category' && selectedCategory ? selectedCategory : null
        );
        // `stream.provider` rides along, so startStream routes to the right
        // platform without the caller having to know which one it is.
        startStream(stream.user_login, stream);
    };

    // Follows for platforms that expose no follow list to us: the channel goes
    // into StreamNook's own list, and the backend poller starts reporting it.
    // Read from the subscribed list (not getState) so the heart fills the moment
    // it is toggled rather than on the next unrelated render.
    const isProviderFollowed = (stream: TwitchStream) => {
        const provider = streamProvider(stream);
        // One canonical identity both ways: the same followIdentifier the write
        // side stores (YouTube's UC id, everyone else's login/slug). Comparing
        // user_login here could never read back a YouTube follow at all.
        const id = followIdentifier(stream);
        const key = provider === 'youtube' ? id : id.toLowerCase();
        return providerFollows.some((f) =>
            f.provider === provider &&
            (f.channel === key ||
                // Legacy: Kick follows written before the identity fix were
                // keyed by the numeric user id; read them as followed so the
                // control tells the truth until the entry is re-toggled.
                (provider === 'kick' && !!stream.user_id && f.channel === stream.user_id)),
        );
    };

    const handleProviderFollowClick = (e: React.MouseEvent, stream: TwitchStream) => {
        e.stopPropagation();
        const provider = streamProvider(stream);
        const follows = useFollowsStore.getState();
        // The CHANNEL, not the broadcast. A YouTube grid row is addressed by video
        // id, so following `user_login` here followed a single stream — and could
        // never match the `UC` ids the subscriptions import writes, which is why
        // the heart and the player disagreed about the same channel.
        const target = followIdentifier(stream);
        const legacyKickId =
            provider === 'kick' && stream.user_id && stream.user_id !== target
                ? stream.user_id
                : null;
        if (follows.isFollowed(provider, target) || (legacyKickId && follows.isFollowed(provider, legacyKickId))) {
            void follows.unfollow(provider, target);
            // Also remove the numeric-keyed legacy form, or the entry the old
            // bug wrote would survive the unfollow and re-read as followed.
            if (legacyKickId) void follows.unfollow(provider, legacyKickId);
        } else {
            void follows.follow(provider, target, stream.user_name);
        }
    };

    // Pending heart-break timers, keyed by favourite id.
    //
    // Un-favouriting is deferred a second so the break animation can play, and
    // without this a re-favourite inside that window left the old timer to fire
    // and toggle the channel straight back off. Reachable before; now that the
    // heart is on every card it is easy to hit.
    const heartBreakTimers = useRef(new Map<string, ReturnType<typeof setTimeout>>());
    useEffect(() => {
        const timers = heartBreakTimers.current;
        return () => {
            for (const t of timers.values()) clearTimeout(t);
            timers.clear();
        };
    }, []);

    const handleFavoriteClick = async (e: React.MouseEvent, stream: TwitchStream) => {
        e.stopPropagation();

        let id = favoriteIdOf(stream);
        if (!id) {
            // Only a YouTube row with no channel identity gets here: it is
            // addressed by video id, which names one broadcast and would sit in
            // the list resolving a finished stream forever. Resolve the channel
            // rather than persisting something that can never match.
            try {
                const meta = await invoke<{ user_id?: string }>('provider_channel_meta', {
                    provider: 'youtube',
                    channel: stream.user_login,
                });
                if (meta?.user_id) id = makeKey('youtube', meta.user_id);
            } catch (err) {
                Logger.warn('[favorites] could not resolve a channel for this row:', err);
            }
            if (!id) {
                useAppStore.getState().addToast("Couldn't work out which channel this is", 'error');
                return;
            }
        }

        const favoriteId = id;
        const pending = heartBreakTimers.current.get(favoriteId);
        if (pending) {
            // A break was already queued for this channel and the user has
            // clicked again, so cancel it: the toggle below is the live intent.
            clearTimeout(pending);
            heartBreakTimers.current.delete(favoriteId);
            setAnimatingHearts(prev => {
                const next = new Set(prev);
                next.delete(favoriteId);
                return next;
            });
        }

        if (isFavoriteStreamer(favoriteId)) {
            setAnimatingHearts(prev => new Set(prev).add(favoriteId));
            const timer = setTimeout(() => {
                heartBreakTimers.current.delete(favoriteId);
                setAnimatingHearts(prev => {
                    const next = new Set(prev);
                    next.delete(favoriteId);
                    return next;
                });
                void toggleFavoriteStreamer(favoriteId);
            }, 1000);
            heartBreakTimers.current.set(favoriteId, timer);
        } else {
            // The identity rides along so the channel can still be drawn once it
            // goes offline: `favorite_streamers` is only ids.
            void toggleFavoriteStreamer(favoriteId, favoriteMetaOf(stream, favoriteId));
        }
    };

    const sortStreamsByFavorites = (streams: TwitchStream[]) => {
        return [...streams].sort((a, b) => {
            // `favoriteIdOf`, not `user_id`: platform ids collide across
            // services, so a Kick row could sort as a Twitch favourite.
            const aId = favoriteIdOf(a);
            const bId = favoriteIdOf(b);
            const aIsFavorite = !!aId && isFavoriteStreamer(aId);
            const bIsFavorite = !!bId && isFavoriteStreamer(bId);

            if (aIsFavorite && !bIsFavorite) return -1;
            if (!aIsFavorite && bIsFavorite) return 1;
            return 0;
        });
    };

    const handleScroll = useCallback(() => {
        const container = scrollContainerRef.current;
        if (!container) return;

        const { scrollTop, scrollHeight, clientHeight } = container;
        const scrollPercentage = (scrollTop + clientHeight) / scrollHeight;

        // Handle recommended streams infinite scroll
        if (activeTab === 'recommended' && hasMoreRecommended && !isLoadingMore && !loadingRef.current) {
            if (scrollPercentage > 0.8) {
                loadingRef.current = true;
                loadMoreRecommendedStreams().finally(() => {
                    loadingRef.current = false;
                });
            }
        }

        // Handle categories (browse) infinite scroll
        if (activeTab === 'browse' && hasMoreGames && !isLoadingMoreGames && !loadingRef.current) {
            if (scrollPercentage > 0.8) {
                loadingRef.current = true;
                loadMoreTopGames().finally(() => {
                    loadingRef.current = false;
                });
            }
        }

        // Handle category content infinite scroll
        if (activeTab === 'category') {
            // The live list is either the server-side tag-filtered set or the
            // Helix category set, depending on whether tags are selected.
            const liveTagMode = selectedCategoryTags.length > 0;
            const liveHasMore = liveTagMode ? hasMoreTagStreams : hasMoreCategoryStreams;
            const liveLoadingMore = liveTagMode ? isLoadingMoreTagStreams : isLoadingMoreCategoryStreams;
            if (categoryActiveTab === 'live' && liveHasMore && !liveLoadingMore && !loadingRef.current) {
                if (scrollPercentage > 0.8) {
                    loadingRef.current = true;
                    (liveTagMode ? loadMoreTagStreams() : loadMoreCategoryStreams()).finally(() => {
                        loadingRef.current = false;
                    });
                }
            } else if (categoryActiveTab === 'clips' && hasMoreCategoryClips && !isLoadingMoreClips && !loadingRef.current) {
                if (scrollPercentage > 0.8) {
                    loadingRef.current = true;
                    loadMoreCategoryClips().finally(() => {
                        loadingRef.current = false;
                    });
                }
            } else if (categoryActiveTab === 'videos' && hasMoreCategoryVideos && !isLoadingMoreVideos && !loadingRef.current) {
                if (scrollPercentage > 0.8) {
                    loadingRef.current = true;
                    loadMoreCategoryVideos().finally(() => {
                        loadingRef.current = false;
                    });
                }
            }
        }
    }, [
        activeTab, hasMoreRecommended, isLoadingMore, loadMoreRecommendedStreams, 
        hasMoreGames, isLoadingMoreGames, loadMoreTopGames, 
        categoryActiveTab, selectedCategoryTags,
        hasMoreCategoryStreams, isLoadingMoreCategoryStreams, loadMoreCategoryStreams,
        hasMoreTagStreams, isLoadingMoreTagStreams, loadMoreTagStreams,
        hasMoreCategoryClips, isLoadingMoreClips, loadMoreCategoryClips,
        hasMoreCategoryVideos, isLoadingMoreVideos, loadMoreCategoryVideos
    ]);

    useEffect(() => {
        const container = scrollContainerRef.current;
        if (!container) return;

        container.addEventListener('scroll', handleScroll);
        return () => container.removeEventListener('scroll', handleScroll);
    }, [handleScroll]);

    // Top-off: if the grid hasn't filled the viewport yet, keep fetching pages
    // until it does. Without this, an initial fetch that's filtered down (e.g.
    // followed channels removed) can leave a partial bottom row with no scroll
    // event to trigger handleScroll.
    useEffect(() => {
        if (activeTab !== 'recommended') return;
        if (!hasMoreRecommended || isLoadingMore || loadingRef.current) return;

        const container = scrollContainerRef.current;
        if (!container) return;

        const fillThreshold = 64; // px slack so we don't loop on near-fills
        if (container.scrollHeight <= container.clientHeight + fillThreshold) {
            loadingRef.current = true;
            loadMoreRecommendedStreams().finally(() => {
                loadingRef.current = false;
            });
        }
    }, [activeTab, recommendedStreams.length, hasMoreRecommended, isLoadingMore, loadMoreRecommendedStreams]);

    // How many live rows to pull PER PLATFORM for the unified Discover grid. The
    // old value of 20 was one YouTube search page, which made Discover look nearly
    // empty next to Twitch's recommendations. YouTube now follows its search
    // continuations to fill this, so the cost is a few requests, not one per row.
    const DISCOVER_PER_PROVIDER = 100;

    // Every other platform's live streams + categories, loaded for the unified
    // view so Discover and Categories aren't silently Twitch-only there.
    const [unifiedProviderStreams, setUnifiedProviderStreams] = useState<TwitchStream[]>([]);
    const [unifiedCategories, setUnifiedCategories] = useState<ProviderCategory[]>([]);
    useEffect(() => {
        if (!isUnifiedView) {
            setUnifiedProviderStreams([]);
            setUnifiedCategories([]);
            return;
        }
        let cancelled = false;
        const others = WATCHABLE_PROVIDERS.filter((p) => p !== 'twitch');
        // Per-platform failures must not empty the merged view, so each is
        // settled independently and a rejection contributes nothing.
        Promise.all(
            others.map((p) =>
                invoke<{ streams: TwitchStream[] }>('provider_directory', { provider: p, limit: DISCOVER_PER_PROVIDER })
                    .then((page) => page.streams ?? [])
                    .catch(() => [] as TwitchStream[]),
            ),
        ).then((pages) => {
            if (!cancelled) setUnifiedProviderStreams(pages.flat());
        });
        Promise.all(
            others.map((p) =>
                invoke<{ categories: ProviderCategory[] }>('provider_categories', { provider: p, limit: 20 })
                    .then((page) => page.categories ?? [])
                    .catch(() => [] as ProviderCategory[]),
            ),
        ).then((pages) => {
            if (!cancelled) setUnifiedCategories(pages.flat());
        });
        return () => {
            cancelled = true;
        };
    }, [isUnifiedView]);

    // The Categories tab shows category TILES until one is picked; every other
    // tab (and a drilled-into category) shows a stream grid.
    const showsProviderCategories =
        isProviderView && activeTab === 'browse' && !providerCategory;

    // Load the platform's categories for the tile grid.
    useEffect(() => {
        if (!showsProviderCategories) return;
        const provider = providerFilter as ProviderId;
        let cancelled = false;
        setIsLoadingProvider(true);
        setProviderError(null);
        invoke<{ categories: ProviderCategory[] }>('provider_categories', { provider, limit: 40 })
            .then((page) => {
                if (!cancelled) setProviderCategories(page.categories ?? []);
            })
            .catch((e) => {
                if (cancelled) return;
                Logger.warn(`[Home] ${provider} categories failed:`, e);
                setProviderCategories([]);
                setProviderError(String(e));
            })
            .finally(() => {
                if (!cancelled) setIsLoadingProvider(false);
            });
        return () => {
            cancelled = true;
        };
    }, [showsProviderCategories, providerFilter]);

    // Leaving the platform view (or switching platform) drops the drill-down so
    // the tab reopens on the tile grid rather than a stale category.
    useEffect(() => {
        setProviderCategory(null);
    }, [providerFilter]);

    // The provider category drill-down lives INSIDE the Categories tab: it is
    // `providerCategory` being set while `activeTab === 'browse'`. Leaving that tab
    // has to clear it.
    //
    // Without this, Discover kept fetching the drilled-in category, because the
    // directory request passes `providerCategory?.id` and BOTH tabs render the same
    // `providerStreams` list on a provider view. Discover and the category view
    // showed an identical grid, and the "All categories" header that would have
    // explained it is itself gated to the Categories tab, so nothing on screen
    // hinted at why. Done as an effect rather than in the tab's onClick so every
    // route out of the tab clears it, not just the one button.
    useEffect(() => {
        if (activeTab !== 'browse' && providerCategory) setProviderCategory(null);
    }, [activeTab, providerCategory]);

    // The drill-down only applies on the Categories tab. Derived rather than read
    // straight from state because the clearing effect above runs AFTER render, so
    // for one render the stale category would still be live and the directory would
    // fire a request for it that is immediately thrown away.
    const activeProviderCategory = activeTab === 'browse' ? providerCategory : null;

    // Fetch the selected platform's directory. Deliberately does NOT depend on
    // the followed-live snapshot: that object changes identity on every poller
    // tick, which would re-request the directory every minute.
    const showsProviderDirectory =
        isProviderView
        && activeTab !== 'following'
        && activeTab !== 'search'
        && !showsProviderCategories;
    useEffect(() => {
        if (!showsProviderDirectory) {
            setProviderStreams([]);
            setProviderError(null);
            // Clear the spinner too. Leaving it set on this path is what made the
            // Following tab load forever until you switched tabs and back — the
            // flag was raised by a previous tab and nothing ever lowered it.
            setIsLoadingProvider(false);
            return;
        }
        const provider = providerFilter as ProviderId;
        let cancelled = false;
        setIsLoadingProvider(true);
        setProviderError(null);
        invoke<{ streams: TwitchStream[] }>('provider_directory', {
            provider,
            category: activeProviderCategory?.id || null,
            // Kick's sorted directory endpoint has no cursor and caps at 100, so
            // there is no second page to fetch — take the whole thing at once
            // rather than showing 40 and silently never loading more.
            limit: 100,
        })
            .then((page) => {
                if (cancelled) return;
                setProviderStreams(page.streams ?? []);
            })
            .catch((e) => {
                if (cancelled) return;
                Logger.warn(`[Home] ${provider} directory failed:`, e);
                setProviderStreams([]);
                setProviderError(String(e));
            })
            .finally(() => {
                if (!cancelled) setIsLoadingProvider(false);
            });
        return () => {
            cancelled = true;
        };
    }, [showsProviderDirectory, providerFilter, activeProviderCategory]);

    // The platform's Following tab reads the poller's snapshot directly, so it
    // needs no request of its own and stays in step with the sidebar. Scoped to
    // one platform it filters to that one; unified, it takes them all.
    // Not gated on the active tab: the Following badge needs this count while
    // you're standing on Discover. It's a filter over an in-memory snapshot, so
    // computing it always is free.
    const providerFollowedLive = useMemo(
        () =>
            !(isProviderView || isUnifiedView)
                ? []
                : Object.values(providerFollowsLive)
                      .filter((row) => row.is_live && (isUnifiedView || row.provider === providerFilter))
                      .sort((a, b) => b.viewer_count - a.viewer_count),
        [isProviderView, isUnifiedView, providerFilter, providerFollowsLive],
    );

    // Composite keys of the provider channels the Following tab is showing, so
    // Discover can leave them out instead of listing the same channel twice.
    const followedProviderKeys = useMemo(
        () => new Set(providerFollowedLive.map((s) => streamKey(s))),
        [providerFollowedLive],
    );

    // Live FAVOURITES, merged from all three places a live row can come from:
    // your Twitch follows, the provider follow poller, and the favourites sweep
    // (which covers channels you follow nowhere). Deduped on the FAVOURITE id
    // rather than `streamKey`, because on YouTube the same channel arrives keyed
    // by video id from a browse row and by UC id from a live check, and a
    // streamKey dedupe lets both through.
    const liveFavorites = useMemo(() => {
        const merged = dedupeByFavoriteId([
            ...followedStreams,
            ...providerFollowedLive,
            ...Object.values(favoritesLive),
        ]);
        return merged
            .filter((s) => {
                const id = favoriteIdOf(s);
                return !!id && isFavoriteStreamer(id);
            })
            .filter((s) => providerFilter === 'all' || streamProvider(s) === providerFilter)
            .sort((a, b) => (b.viewer_count ?? 0) - (a.viewer_count ?? 0));
        // `favoriteStreamers` is tracked so the list re-derives when a heart is
        // toggled; `isFavoriteStreamer` reads it but isn't itself reactive.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [followedStreams, providerFollowedLive, favoritesLive, favoriteStreamers, providerFilter]);

    const favoriteKeys = useMemo(
        () => new Set(liveFavorites.map((s) => streamKey(s))),
        [liveFavorites],
    );

    const rawDisplayStreams = isProviderView
        ? (activeTab === 'following'
            ? providerFollowedLive
            // Without this the search tab rendered the platform DIRECTORY, which is
            // why searching a name appeared to return everything.
            : activeTab === 'search'
              ? searchResults
              : providerStreams)
        : activeTab === 'following'
        // Unified: your Twitch follows and every other platform's live follows in
        // one list. Favorites still float to the top; the rest rank by viewers.
        ? sortStreamsByFavorites([...followedStreams, ...providerFollowedLive])
        : activeTab === 'recommended'
            // Unified Discover: Twitch's personalized picks plus what's big on the
            // other platforms right now, ranked together.
            // Unified Discover: Twitch's picks plus the other platforms' live
            // streams, ranked together by viewers, same as the Twitch-only list.
            ? (isUnifiedView
                // Minus anything the Following tab is already showing you.
                // Twitch's recommendations exclude your follows upstream, but a
                // platform DIRECTORY is just "who is live, ranked", so a Kick
                // channel you follow was turning up on both tabs.
                ? [
                    ...recommendedStreams,
                    ...unifiedProviderStreams.filter((s) => !followedProviderKeys.has(streamKey(s))),
                  ].sort((a, b) => b.viewer_count - a.viewer_count)
                : recommendedStreams)
            : activeTab === 'category'
                ? categoryStreams
                : searchResults.filter(s => s.viewer_count > 0 || s.is_live);

    // Deduplicate on the SAME key the grid renders with, so a channel that
    // arrives from two sources (a paginated page overlapping the previous one, or
    // a stream present in both a follow list and a directory) can never mount
    // twice. React's duplicate-key warning is the visible symptom; the real
    // hazard is rows sharing identity and swapping content during updates.
    const displayStreams = useMemo(() => {
        const seen = new Set<string>();
        return rawDisplayStreams.filter((s) => {
            // The COMPOSITE key, never the bare platform id: Twitch and Kick both
            // use numeric ids, so `id` alone collides across platforms in the
            // mixed view (which is exactly where the duplicate-key warning fired).
            const key = streamKey(s);
            if (seen.has(key)) return false;
            // Favourites are pulled out into their own section above on the tabs
            // that show one; leaving them here too would render each one twice.
            if ((activeTab === 'following' || activeTab === 'recommended') && favoriteKeys.has(key)) {
                return false;
            }
            seen.add(key);
            return true;
        });
    }, [rawDisplayStreams, activeTab, favoriteKeys]);

    // Channel avatars for the cards on screen. A stream row does not reliably carry
    // one: Twitch needs a Helix users lookup, and YouTube category rows need a
    // per-channel resolve (search and the subscriptions feed already ship theirs).


    // ONE card renderer, used by both the main grid and the Favourites
    // section above it. Extracted rather than copied: the Categories tab
    // already has its own duplicate of this card, and a third copy would be
    // a third place for the two to drift apart.
    const renderStreamCard = (stream: TwitchStream) => {
                                        // May be null for a YouTube row with no channel identity;
                                        // the click handler resolves one before writing.
                                        const favoriteId = favoriteIdOf(stream);
                                        const isFavorite = !!favoriteId && isFavoriteStreamer(favoriteId);
                                        // Check if stream's game has active drops
                                        const streamDropsCampaign = stream.game_name ? dropsGameNames.get(stream.game_name.toLowerCase()) : undefined;
                                        const hasDrops = !!streamDropsCampaign;
                                        return (() => {
                                            const isQueued = isInMultiNook(stream.user_login, streamProvider(stream));
                                            const isSuckingUp = suckUpKey === makeKey(streamProvider(stream), stream.user_login);
                                            const isMaterializing = materializingKey === makeKey(streamProvider(stream), stream.user_login);

                                            return (
                                                <motion.div
                                                    layout
                                                    // Opacity and a short lift, never scale: a
                                                    // scaling element fights `layout`'s own scale
                                                    // correction and the card's text and rounded
                                                    // corners smear while it settles.
                                                    initial={isReopen ? false : { opacity: 0, y: 8 }}
                                                    animate={{ opacity: 1, y: 0 }}
                                                    exit={{ opacity: 0, y: -4 }}
                                                    transition={{
                                                        type: "spring",
                                                        stiffness: 350,
                                                        damping: 25,
                                                        opacity: { duration: 0.16, ease: 'easeOut' },
                                                        y: { duration: 0.2, ease: [0.2, 0.9, 0.25, 1] },
                                                    }}
                                                    // Composite provider:channel key. Not `stream.id` — provider rows
                                                    // may carry none, and where they do the numeric ids collide with
                                                    // Twitch's in the mixed view.
                                                    key={streamKey(stream)}
                                                    // Shared layout identity: hearting a card re-parents it
                                                    // between the Favorites grid and the follows grid, and the
                                                    // shared LayoutGroup around both makes that a glide from old
                                                    // slot to new slot instead of a fade-out/fade-in.
                                                    layoutId={`card-${streamKey(stream)}`}
                                                    data-avatar-key={streamKey(stream)}
                                                    className={`p-2.5 transition-all duration-200 group relative ${
                                                        isQueued && !isSuckingUp
                                                            ? 'ghost-card rounded-lg cursor-default'
                                                            : isQueued && isSuckingUp
                                                                ? `glass-panel media-card cursor-default ${isOverlayMode ? '!bg-black/40 !border-white/5' : ''}`
                                                                : `glass-panel media-card cursor-pointer hover:bg-glass-hover ${isOverlayMode ? '!bg-black/40 !border-white/5' : ''} ${stream.has_shared_chat === true ? 'iridescent-border' : ''}`
                                                    }`}
                                                    onClick={(e) => !isQueued && handleStreamClick(e, stream)}
                                                    onContextMenu={(e) => !isQueued && useContextMenuStore.getState().openMenu(e, stream)}
                                                >
                                                    {isQueued && !isSuckingUp ? (
                                                        /* Ghost state — recall button + label */
                                                        <>
                                                            <div className="invisible">
                                                                <div className="relative mb-2 overflow-hidden rounded aspect-video" />
                                                                <div className="flex items-end justify-between mt-1">
                                                                    <div className="space-y-0.5 flex-1 min-w-0 pr-2 pb-1">
                                                                        <div className="h-4" />
                                                                        <div className="h-3" />
                                                                        <div className="h-3" />
                                                                    </div>
                                                                </div>
                                                            </div>
                                                            <div className="absolute inset-0 flex flex-col items-center justify-center gap-1.5 animate-ghost-label">
                                                                <LayoutGrid size={18} className="text-accent/50" />
                                                                <span className="text-accent text-xs font-semibold truncate max-w-[80%]">{stream.user_name}</span>
                                                                <span className="text-textSecondary text-[10px]">Queued in MultiNook</span>
                                                                <Tooltip content="Recall from MultiNook" side="bottom">
                                                                    <button
                                                                        onClick={(e) => {
                                                                            e.stopPropagation();
                                                                            const card = (e.currentTarget as HTMLElement).closest('.ghost-card');
                                                                            const rect = card?.getBoundingClientRect();
                                                                            const cx = rect ? rect.left + rect.width / 2 : e.clientX;
                                                                            const cy = rect ? rect.top + rect.height / 2 : e.clientY;
                                                                            triggerRecallAnimation(stream.user_login, cx, cy, streamProvider(stream));
                                                                        }}
                                                                        className="glass-button !rounded-full !p-1.5 mt-1 text-textSecondary hover:text-accent transition-colors"
                                                                    >
                                                                        <Undo2 size={14} strokeWidth={2} />
                                                                    </button>
                                                                </Tooltip>
                                                            </div>
                                                        </>
                                                    ) : (
                                                        /* Normal content, suck-up, or materialize animation */
                                                        <div className={isSuckingUp ? 'animate-multinook-suck-up' : isMaterializing ? 'animate-multinook-materialize' : undefined}>
                                                            {!isSuckingUp && <QuickAddButton stream={stream} />}
                                                            <div className="relative mb-2 overflow-hidden rounded">
                                                                <img
                                                                    loading="lazy"
                                                                    src={getThumbnailUrl(stream.thumbnail_url)}
                                                                    alt={stream.title}
                                                                    className="w-full aspect-video object-cover group-hover:scale-105 transition-transform duration-200"
                                                                />
                                                                <div className="absolute top-1.5 left-1.5 flex items-center gap-1">
                                                                    <div className="live-dot text-xs px-1.5 py-0.5">LIVE</div>
                                                                    {hasDrops && (
                                                                        <div className="drops-badge-glass">
                                                                            <Gift size={10} />
                                                                            <span>DROPS</span>
                                                                        </div>
                                                                    )}
                                                                    {activeHypeTrainChannels.get(stream.user_id) && (
                                                                        <div className={activeHypeTrainChannels.get(stream.user_id)?.isGolden ? 'hype-train-badge-glass-golden' : 'hype-train-badge-glass'}>
                                                                            <svg className="w-2.5 h-2.5" viewBox="0 0 15 13" fill="none">
                                                                                <path fillRule="evenodd" clipRule="evenodd" d="M4.10001 0.549988H2.40001V4.79999H0.700012V10.75H1.55001C1.55001 11.6889 2.31113 12.45 3.25001 12.45C4.1889 12.45 4.95001 11.6889 4.95001 10.75H5.80001C5.80001 11.6889 6.56113 12.45 7.50001 12.45C8.4389 12.45 9.20001 11.6889 9.20001 10.75H10.05C10.05 11.6889 10.8111 12.45 11.75 12.45C12.6889 12.45 13.45 11.6889 13.45 10.75H14.3V0.549988H6.65001V2.24999H7.50001V4.79999H4.10001V0.549988ZM12.6 9.04999V6.49999H2.40001V9.04999H12.6ZM9.20001 4.79999H12.6V2.24999H9.20001V4.79999Z" fill="currentColor" />
                                                                            </svg>
                                                                            <span>LVL {activeHypeTrainChannels.get(stream.user_id)?.level}</span>
                                                                        </div>
                                                                    )}
                                                                </div>
                                                                <div className="absolute bottom-1.5 left-1.5 flex items-center gap-1">
                                                                    <div className="px-2 py-0.5 glass-badge text-white text-xs font-medium rounded">
                                                                        {stream.viewer_count.toLocaleString()} viewers
                                                                    </div>
                                                                </div>
                                                                {/* Bottom-right corner: watch streak and platform mark share
                                                                    one row so they can never overlap. */}
                                                                <div className="absolute bottom-1.5 right-1.5 flex items-center gap-1">
                                                                    {watchStreaks[stream.user_id] > 0 && (
                                                                        <Tooltip content={`${watchStreaks[stream.user_id]} Stream Watch Streak`} side="top">
                                                                        <div className="flex items-center gap-1 font-bold text-[10px] leading-tight px-1.5 py-0.5 rounded shadow-[0_0_10px_color-mix(in_srgb,var(--color-warning)_25%,transparent)] bg-amber-500/10 text-amber-400 border border-amber-500/30 backdrop-blur-md">
                                                                            <Flame size={10} className="stroke-[2.5]" />
                                                                            <span>{watchStreaks[stream.user_id]}</span>
                                                                        </div>
                                                                        </Tooltip>
                                                                    )}
                                                                </div>
                                                            </div>
                                                            <div className="flex items-end justify-between mt-1">
                                                                <div className="space-y-0.5 flex-1 min-w-0 pr-2 pb-1">
                                                                    <h3 className="text-textPrimary font-medium text-sm line-clamp-1 group-hover:text-accent transition-colors">
                                                                        <StreamTitleWithEmojis title={stream.title} />
                                                                    </h3>
                                                                    <button 
                                                                        onClick={(e) => { 
                                                                            e.stopPropagation(); 
                                                                            useAppStore.getState().setProfileModalUser(stream); 
                                                                        }}
                                                                        className="flex items-center gap-1 text-textSecondary text-xs hover:text-textPrimary hover:bg-glass-hover px-1.5 py-0.5 -mx-1.5 -my-0.5 rounded transition-all cursor-pointer text-left focus:outline-none w-max max-w-full"
                                                                    >
                                                                        {/* Channel avatar. Resolved per platform; falls back to a
                                                                            monogram rather than a broken image or a foreign
                                                                            platform's default picture. */}
                                                                        {/* No placeholder when a platform ships no avatar: a row of
                                                                            identical grey monograms is noise, not information. */}
                                                                        {(stream.profile_image_url || cardAvatars[streamKey(stream)]) && (
                                                                            <img
                                                                                loading="lazy"
                                                                                src={stream.profile_image_url || cardAvatars[streamKey(stream)]}
                                                                                alt=""
                                                                                className="w-4 h-4 rounded-full object-cover flex-shrink-0 ring-1 ring-borderSubtle"
                                                                            />
                                                                        )}
                                                                        <span className="truncate">{stream.user_name}</span>
                                                                        {stream.broadcaster_type === 'partner' && (
                                                                            <svg className="w-3 h-3 flex-shrink-0" viewBox="0 0 16 16" fill="#9146FF">
                                                                                <path fillRule="evenodd" d="M12.5 3.5 8 2 3.5 3.5 2 8l1.5 4.5L8 14l4.5-1.5L14 8l-1.5-4.5ZM7 11l4.5-4.5L10 5 7 8 5.5 6.5 4 8l3 3Z" clipRule="evenodd"></path>
                                                                            </svg>
                                                                        )}
                                                                    </button>
                                                                    {/* min-h reserves this line even when a platform sends no
                                                                        category (YouTube), so the bottom-anchored platform mark
                                                                        sits on the same baseline on every card in the row
                                                                        instead of riding up on shorter info blocks. */}
                                                                    <div className="flex items-center w-full min-h-4">
                                                                        {stream.game_name && (
                                                                        <Tooltip content={stream.game_name} side="bottom">
                                                                            <button 
                                                                                onClick={(e) => {
                                                                                    e.stopPropagation();
                                                                                    if (stream.game_id && stream.game_name) {
                                                                                        handleCategoryClick({ 
                                                                                            id: stream.game_id, 
                                                                                            name: stream.game_name, 
                                                                                            box_art_url: '' 
                                                                                        });
                                                                                    }
                                                                                }}
                                                                                className="flex items-center gap-1 text-textMuted text-xs hover:text-textPrimary hover:bg-glass-hover px-1.5 py-0.5 -mx-1.5 -my-0.5 rounded transition-all text-left cursor-pointer focus:outline-none overflow-hidden"
                                                                            >
                                                                                <span className="line-clamp-1">{stream.game_name}</span>
                                                                                {hasDrops && (
                                                                                    <Gift size={10} className="text-accent flex-shrink-0" />
                                                                                )}
                                                                            </button>
                                                                        </Tooltip>
                                                                        )}
                                                                    </div>
                                                                </div>

                                                                {/* Follow toggle for platforms with no follow API of
                                                                    their own — StreamNook keeps the list. Shown on every
                                                                    tab, since the platform's directory is where you find
                                                                    channels to follow in the first place. */}
                                                                {/* FOLLOW, and only where following is the question — a
                                                                    platform's directory or search. On the Following tab every
                                                                    row is already followed, so a filled heart on each would
                                                                    say nothing; that tab gets the favourite toggle instead,
                                                                    exactly like Twitch's. Plus (not a heart) so it never reads
                                                                    as the favourite control. */}
                                                                {isProviderView && activeTab !== 'following' && (
                                                                    <Tooltip content={isProviderFollowed(stream) ? `Unfollow on ${providerLabel(streamProvider(stream))}` : `Follow on ${providerLabel(streamProvider(stream))}`} side="top">
                                                                    <button
                                                                        onClick={(e) => handleProviderFollowClick(e, stream)}
                                                                        className="p-1 flex items-center justify-center bg-transparent transition-transform duration-300 hover:scale-110 active:scale-95"
                                                                    >
                                                                        {isProviderFollowed(stream) ? (
                                                                            <Check size={16} className="text-accent" strokeWidth={2.5} />
                                                                        ) : (
                                                                            <Plus size={16} className="text-textSecondary hover:text-textPrimary opacity-0 group-hover:opacity-100 transition-all duration-300" strokeWidth={2.5} />
                                                                        )}
                                                                    </button>
                                                                    </Tooltip>
                                                                )}
                                                                {/* FAVOURITE, on every tab. A favourite is a personal
                                                                    watchlist entry, not a re-ordering of your follows:
                                                                    the whole point is hearting something you found in
                                                                    Discover or a category and having it turn up when it
                                                                    goes live, without following the channel. On a
                                                                    provider directory row this sits beside the follow
                                                                    control above, which is correct - they are different
                                                                    actions - and the heart is always the rightmost. */}
                                                                <Tooltip content={isFavorite ? 'Remove from favorites' : 'Add to favorites'} side="top">
                                                                <button
                                                                    onClick={(e) => { void handleFavoriteClick(e, stream); }}
                                                                    className={`p-1 flex items-center justify-center bg-transparent transition-transform duration-300 hover:scale-110 active:scale-95`}
                                                                >
                                                                    <Heart
                                                                        size={16}
                                                                        fill={isFavorite ? "url(#glass-heart-fill)" : "none"}
                                                                        stroke={isFavorite ? "url(#glass-heart-stroke)" : "currentColor"}
                                                                        strokeWidth={isFavorite ? 1.5 : 2}
                                                                        className={`transition-all duration-300 ${isFavorite ? 'drop-shadow-[0_4px_8px_color-mix(in_srgb,var(--color-highlight-pink)_50%,transparent)]' : 'text-textSecondary hover:text-textPrimary opacity-0 group-hover:opacity-100'} ${favoriteId && animatingHearts.has(favoriteId) ? 'animate-heart-break' : ''}`}
                                                                    />
                                                                </button>
                                                                </Tooltip>
                                                                {/* Platform, in the card's bottom-right corner — on the card
                                                                    itself, not over the preview, where it would compete with
                                                                    the artwork. Bare mark, no chip or button: at this size
                                                                    these read by colour, and a container would make a passive
                                                                    label look clickable. Shown ONLY while the grid is mixed,
                                                                    and then on EVERY card including Twitch, since with
                                                                    platforms mixed an unmarked card is a guess. */}
                                                                {isUnifiedView && (
                                                                    <Tooltip content={providerLabel(streamProvider(stream))} side="top">
                                                                        {/* Same 24px box the heart button uses (p-1 + 16px icon),
                                                                            mark centered, so the two icons share one center line
                                                                            instead of mixing padded-box and bottom-padded
                                                                            geometries. Cross-card alignment comes from the
                                                                            reserved category line, so center optical mode is
                                                                            right here. */}
                                                                        <span className="flex h-6 w-6 flex-shrink-0 items-center justify-center opacity-80">
                                                                            <ProviderLogo provider={streamProvider(stream)} size={13} />
                                                                        </span>
                                                                    </Tooltip>
                                                                )}
                                                            </div>
                                                        </div>
                                                    )}
                                                </motion.div>
                                            );
                                        })();
    };

    // ONE offline-channel card, shared by the Offline Channels roster and the
    // Favourites section (a favourite is usually NOT live, and burying it in a
    // roster of hundreds of follows is indistinguishable from doing nothing).
    const renderOfflineCard = (user: TwitchStream) => {
                                                const lastOnline = offlineLastBroadcasts[user.id];
                                                let relativeTimeResult = '';
                                                if (lastOnline) {
                                                    const date = new Date(lastOnline);
                                                    if (!isNaN(date.getTime())) {
                                                        const diffInSeconds = Math.floor((new Date().getTime() - date.getTime()) / 1000);
                                                        if (diffInSeconds < 60) relativeTimeResult = `${diffInSeconds}s ago`;
                                                        else if (diffInSeconds < 3600) relativeTimeResult = `${Math.floor(diffInSeconds / 60)}m ago`;
                                                        else if (diffInSeconds < 86400) relativeTimeResult = `${Math.floor(diffInSeconds / 3600)}h ago`;
                                                        else if (diffInSeconds < 2592000) relativeTimeResult = `${Math.floor(diffInSeconds / 86400)}d ago`;
                                                        else if (diffInSeconds < 31536000) relativeTimeResult = `${Math.floor(diffInSeconds / 2592000)}mo ago`;
                                                        else relativeTimeResult = `${Math.floor(diffInSeconds / 31536000)}y ago`;
                                                    }
                                                }

                                                return (
                                                    <div
                                                        // Composite key: this list now mixes platforms, and a bare
                                                        // platform id can collide across them.
                                                        key={streamKey(user)}
                                                        // Tags this card for the visibility observer that drives
                                                        // avatar resolution.
                                                        data-avatar-key={streamKey(user)}
                                                        className="relative group w-[180px] sm:w-[200px] rounded-xl overflow-hidden glass-panel border border-borderSubtle hover:border-white/20 transition-all shadow-sm"
                                                    >
                                                        {/* Base Card Content */}
                                                        <div className="w-full flex items-center gap-3 px-3 py-2">
                                                            <div className="w-10 h-10 rounded-full bg-glass flex items-center justify-center overflow-hidden ring-1 ring-borderSubtle group-hover:ring-accent/40 flex-shrink-0 relative transition-all">
                                                                {(() => {
                                                                    // Twitch rows carry a thumbnail; provider follows carry
                                                                    // nothing, so their avatar comes from the resolver.
                                                                    const offlineAvatar =
                                                                        user.profile_image_url ||
                                                                        cardAvatars[streamKey(user)] ||
                                                                        user.thumbnail_url;
                                                                    return offlineAvatar ? (
                                                                        <img src={offlineAvatar} alt={user.user_name} className="w-full h-full object-cover" />
                                                                    ) : (
                                                                        <User size={14} className="text-textSecondary" />
                                                                    );
                                                                })()}
                                                            </div>
                                                            <div className="flex-1 min-w-0 transition-opacity duration-200">
                                                                <h4 className="text-sm font-semibold text-textPrimary truncate transition-colors">
                                                                    {user.user_name}
                                                                </h4>
                                                                <p className="text-[10px] text-textSecondary truncate">
                                                                    {relativeTimeResult ? `Last live ${relativeTimeResult}` : 'Offline'}
                                                                </p>
                                                            </div>
                                                        </div>

                                                        {/* Favourite, on the offline card too. Unlike the actions
                                                            below this is NOT Twitch-only: favouriting works on every
                                                            platform, and a channel found while it is offline (a search
                                                            result, or one already in this roster) is exactly the kind
                                                            you want to be told about when it comes back. Sits above the
                                                            hover overlay so it stays clickable. */}
                                                        {(() => {
                                                            const offlineFavoriteId = favoriteIdOf(user);
                                                            const offlineIsFavorite = !!offlineFavoriteId && isFavoriteStreamer(offlineFavoriteId);
                                                            return (
                                                                <Tooltip content={offlineIsFavorite ? 'Remove from favorites' : 'Add to favorites'} side="top">
                                                                    <button
                                                                        onClick={(e) => { void handleFavoriteClick(e, user); }}
                                                                        className="absolute top-1 right-1 z-20 p-1 flex items-center justify-center bg-transparent transition-transform duration-300 hover:scale-110 active:scale-95"
                                                                    >
                                                                        <Heart
                                                                            size={14}
                                                                            fill={offlineIsFavorite ? "url(#glass-heart-fill)" : "none"}
                                                                            stroke={offlineIsFavorite ? "url(#glass-heart-stroke)" : "currentColor"}
                                                                            strokeWidth={offlineIsFavorite ? 1.5 : 2}
                                                                            className={`transition-all duration-300 ${offlineIsFavorite ? 'drop-shadow-[0_4px_8px_color-mix(in_srgb,var(--color-highlight-pink)_50%,transparent)]' : 'text-textSecondary hover:text-textPrimary opacity-0 group-hover:opacity-100'} ${offlineFavoriteId && animatingHearts.has(offlineFavoriteId) ? 'animate-heart-break' : ''}`}
                                                                        />
                                                                    </button>
                                                                </Tooltip>
                                                            );
                                                        })()}

                                                        {/* Action Overlay (Optimized - No blur on hidden elements).
                                                            Twitch only: "Watch VOD" starts Twitch's offline-chat mode
                                                            and the profile modal is Twitch-shaped, so neither works for
                                                            a provider channel. Better no action than a broken one. */}
                                                        {streamProvider(user) === 'twitch' && (
                                                        <div className="absolute inset-0 bg-[#0c0c0d]/90 opacity-0 group-hover:opacity-100 transition-all duration-200 flex items-center justify-center gap-2 z-10">
                                                            <button
                                                                onClick={(e) => {
                                                                    e.stopPropagation();
                                                                    const store = useAppStore.getState();
                                                                    if (store.isHomeActive) store.toggleHome();
                                                                    store.startOfflineChat(user.user_login, user);
                                                                }}
                                                                className="px-3 py-1.5 rounded-lg glass-button text-[12px] font-bold text-white hover:bg-white/20 transition-all border border-transparent shadow-lg flex items-center gap-1.5"
                                                            >
                                                                <Play size={14} strokeWidth={2.5} />
                                                                <span>Watch VOD</span>
                                                            </button>
                                                            
                                                            <Tooltip content="Profile" side="top">
                                                                <button
                                                                    onClick={(e) => {
                                                                        e.stopPropagation();
                                                                        setProfileModalUser(user);
                                                                    }}
                                                                    className="p-[7px] rounded-lg glass-button text-textSecondary hover:text-textPrimary hover:bg-white/20 transition-all border border-transparent shadow-lg"
                                                                >
                                                                    <User size={14} strokeWidth={2.5} />
                                                                </button>
                                                            </Tooltip>
                                                        </div>
                                                        )}
                                                    </div>
                                                );
    };

    // How many live channels the Following tab holds for the CURRENT platform.
    const followingCount = isProviderView
        ? providerFollowedLive.length
        : isUnifiedView
            ? followedStreams.length + providerFollowedLive.length
            : followedStreams.length;

    const offlineSearchResults = activeTab === 'search' ? searchResults.filter(s => s.viewer_count === 0 && !s.is_live) : [];

    // Dropdown tags are the category's own offered tags only (not the free-form
    // tags individual streamers set). The text search can still match anything.
    const availableCategoryTags = useMemo(() => {
        const seen = new Set<string>();
        const tags: string[] = [];
        categoryDetails?.tags?.forEach(t => {
            const label = t.localizedName.trim();
            const key = label.toLowerCase();
            if (label && !seen.has(key)) {
                seen.add(key);
                tags.push(label);
            }
        });
        return tags;
    }, [categoryDetails]);

    // Broader autocomplete pool: the free-form tags carried by the loaded live
    // streams. Folded into the tag search only while typing (not the default list).
    const categoryStreamTags = useMemo(() => {
        const seen = new Set<string>();
        const tags: string[] = [];
        categoryStreams.forEach(s => s.tags?.forEach(t => {
            const label = t.trim();
            const key = label.toLowerCase();
            if (label && !seen.has(key)) {
                seen.add(key);
                tags.push(label);
            }
        }));
        return tags;
    }, [categoryStreams]);

    // The live list is the server-side tag-filtered set when tags are selected,
    // otherwise the Helix category list. Tag matching is done server-side; the
    // search box only picks tags (it doesn't fuzzy-filter the visible streams).
    const tagMode = selectedCategoryTags.length > 0;
    const baseLiveStreams = tagMode ? tagStreams : categoryStreams;

    // Followed provider channels that are NOT live, so the Following tab shows an
    // offline roster for them the way it already does for Twitch.
    //
    // Matching "which follows are live" cannot be done on the composite key: a
    // YouTube live row is keyed by its VIDEO id while a follow is stored under the
    // channel's UC id, so the keys never line up. Both shapes DO agree on the
    // channel though, which is what this compares.
    const offlineProviderFollows = useMemo<TwitchStream[]>(() => {
        if (!(isProviderView || isUnifiedView)) return [];
        const liveChannels = new Set<string>();
        for (const row of Object.values(providerFollowsLive)) {
            if (row.user_id) liveChannels.add(`${row.provider}:${row.user_id.toLowerCase()}`);
            if (row.user_login) liveChannels.add(`${row.provider}:${row.user_login.toLowerCase()}`);
        }
        return providerFollows
            .filter((f) => isUnifiedView || f.provider === providerFilter)
            .filter((f) => !liveChannels.has(`${f.provider}:${f.channel.toLowerCase()}`))
            .map((f) => ({
                id: f.channel,
                // The channel id doubles as the user id so the avatar resolver,
                // which keys off `user_id`, can look it up like any other row.
                user_id: f.channel,
                user_login: f.channel,
                user_name: f.display_name || f.channel,
                provider: f.provider,
                key: makeKey(f.provider, f.channel),
                title: '',
                viewer_count: 0,
                game_id: '',
                game_name: '',
                thumbnail_url: '',
                started_at: '',
                is_live: false,
                // Captured when the follow was imported, so the roster paints
                // immediately instead of resolving each face on every app start.
                // Rows imported before this existed have none and fall through to
                // the per-card resolver as before.
                profile_image_url: f.avatar,
            } as unknown as TwitchStream));
    }, [isProviderView, isUnifiedView, providerFilter, providerFollows, providerFollowsLive]);

    // Favourites that aren't live anywhere, so you can still get back to them.
    //
    // Built from the identity sidecar rather than a live row, because there is
    // no live row: that's the point. Rows whose id has left `favorite_streamers`
    // are filtered out HERE rather than pruned from settings on startup, which
    // would mean a settings write racing every other write during launch.
    const offlineFavorites = useMemo<TwitchStream[]>(() => {
        const identities = favoriteChannels ?? [];
        if (identities.length === 0) return [];
        const favorites = new Set(favoriteStreamers ?? []);
        // What is already live, by favourite id, across all three sources.
        const liveIds = new Set(
            liveFavorites.map((s) => favoriteIdOf(s)).filter((id): id is string => !!id),
        );
        // Prefer a REAL offline row where one exists (a Twitch follow, or a
        // provider follow): it carries the proper id, so the "last live" lookup
        // below resolves, and the avatar resolver keys off it. The sidecar row
        // is the fallback for a favourite followed nowhere, which has no real
        // row anywhere by definition.
        const realRows = new Map<string, TwitchStream>();
        for (const row of [...(isProviderView ? [] : offlineFollowedChannels), ...offlineProviderFollows]) {
            const id = favoriteIdOf(row);
            if (id && !realRows.has(id)) realRows.set(id, row);
        }
        return identities
            .filter((f) => favorites.has(f.id) && !liveIds.has(f.id))
            .filter((f) => providerFilter === 'all' || f.provider === providerFilter)
            .map((f) => realRows.get(f.id) ?? ({
                id: f.channel,
                // For Twitch the sidecar id IS the user id, which is what the
                // avatar resolver and the profile modal key off. Other platforms
                // have no such id here, so the channel stands in, matching what
                // `offlineProviderFollows` already does.
                user_id: f.provider === 'twitch' ? f.id : f.channel,
                user_login: f.channel,
                user_name: f.display_name || f.channel,
                provider: f.provider,
                key: makeKey(f.provider, f.channel),
                title: '',
                viewer_count: 0,
                game_id: '',
                game_name: '',
                thumbnail_url: '',
                started_at: '',
                is_live: false,
                profile_image_url: f.avatar,
            } as unknown as TwitchStream));
    }, [favoriteChannels, favoriteStreamers, liveFavorites, providerFilter, isProviderView, offlineFollowedChannels, offlineProviderFollows]);

    // The offline roster: Twitch's own list (only meaningful when Twitch is in
    // view), followed provider channels that aren't live, and favourites that
    // aren't live anywhere. Deduped on channel identity, since a favourite you
    // also follow is in two of those three lists.
    const offlineChannels = useMemo<TwitchStream[]>(
        // Offline favourites belong HERE, not in the Favourites section: that
        // shelf is live-only. This is also the only way a favourite you follow
        // nowhere is reachable while it is offline, since no follow list carries
        // it. Deduped, because a favourite you also follow is in two of these.
        () => dedupeByFavoriteId([
            ...(isProviderView ? [] : offlineFollowedChannels),
            ...offlineProviderFollows,
            ...offlineFavorites,
        ]),
        [isProviderView, offlineFollowedChannels, offlineProviderFollows, offlineFavorites],
    );

    // Rows the Favourites section will draw. Load-bearing in the empty-state test
    // further down, not just for rendering: favourites are SUBTRACTED from
    // `displayStreams`, so without counting them there, favouriting every live
    // channel emptied `displayStreams`, hit the "No Live Streams" branch, and
    // took the whole Favourites section down with it.
    // Scoped to the Following tab, or a live favourite would suppress the empty
    // state on Discover, where the section does not render at all.
    // Tabs that show the Favourites shelf. Following AND Discover: favouriting
    // is a browsing action, and Discover is where you meet a channel in the
    // first place. It is also the only one of the two a signed-out user can
    // reach at all - Following is a login wall for Twitch and a "Not Connected"
    // wall for every provider - which is exactly the case favourites exist for.
    const favoritesTab = activeTab === 'following' || activeTab === 'recommended';
    const favoritesSectionCount = favoritesTab ? liveFavorites.length : 0;

    // Both grids: the main list and the category-detail list.
    const avatarCandidates = useMemo(
        () => [...displayStreams, ...liveFavorites, ...baseLiveStreams, ...offlineChannels],
        [displayStreams, liveFavorites, baseLiveStreams, offlineChannels],
    );
    // Only the cards actually on screen get resolved. A YouTube avatar costs a
    // channel lookup each, and a large subscription list would otherwise fire
    // hundreds of requests for rows the user never scrolls to.
    const visibleAvatarKeys = useVisibleAvatarKeys([avatarCandidates]);
    const avatarTargets = useMemo(
        () => avatarCandidates.filter((s) => visibleAvatarKeys.has(streamKey(s))),
        [avatarCandidates, visibleAvatarKeys],
    );
    const cardAvatars = useStreamAvatars(avatarTargets);

    const toggleCategoryTag = (label: string) => {
        const key = label.toLowerCase();
        setSelectedCategoryTags(prev =>
            prev.some(t => t.toLowerCase() === key)
                ? prev.filter(t => t.toLowerCase() !== key)
                : [...prev, label]
        );
    };

    const renderCategoryCard = (game: TwitchCategory) => {
        const dropsCampaign = dropsGameIds.get(game.id);
        const hasDrops = !!dropsCampaign;

        return (
            <div
                key={game.id}
                className={`glass-panel media-card cursor-pointer hover:bg-glass-hover transition-all duration-200 group overflow-hidden relative ${isOverlayMode ? '!bg-black/40 !border-white/5' : ''} ${hasDrops ? 'ring-2 ring-accent shadow-accent/40' : ''}`}
                style={hasDrops ? { boxShadow: '0 0 15px var(--color-accent-muted)' } : undefined}
                onClick={() => handleCategoryClick(game)}
            >
                <div className="relative overflow-hidden">
                    <img
                        loading="lazy"
                        src={getGameBoxArt(game.box_art_url)}
                        alt={game.name}
                        className="w-full aspect-[3/4] object-cover group-hover:scale-105 transition-transform duration-200"
                    />
                    {hasDrops && (
                        <div className="absolute inset-0 bg-gradient-to-t from-accent/40 via-transparent to-accent/20 pointer-events-none" />
                    )}
                    {hasDrops && (
                        <div className="absolute top-2 left-2 z-10">
                            <div className="drops-badge-glass-lg">
                                <Gift size={14} className="drop-shadow-lg" />
                                <span>DROPS</span>
                            </div>
                        </div>
                    )}
                    {hasDrops && externalDropsProvider && (
                        <Tooltip content={activeAutomationIds.has(dropsCampaign.id) ? `Click to stop automation ${dropsCampaign.name}` : `Start automation ${dropsCampaign.name}`} side="top">
                        <button
                            onClick={(e) => handleToggleAutomation(e, dropsCampaign)}
                            className={`absolute bottom-2 right-2 left-2 flex items-center justify-center gap-1.5 px-2 py-1.5 rounded-md text-xs font-semibold transition-all duration-300 glass-button ${activeAutomationIds.has(dropsCampaign.id)
                                ? '!bg-success/20 text-success border-success/30 !shadow-[0_2px_10px_color-mix(in_srgb,var(--color-success)_30%,transparent),inset_0_1px_rgba(255,255,255,0.2)] ring-1 ring-success/20 hover:!bg-error/30 hover:text-error hover:border-error/30 hover:ring-error/30 hover:!shadow-[0_2px_10px_color-mix(in_srgb,var(--color-error)_30%,transparent),inset_0_1px_rgba(255,255,255,0.2)]'
                                : '!bg-accent/30 text-white border-accent/40 !shadow-[0_2px_10px_rgba(var(--color-accent-rgb),0.4),inset_0_1px_rgba(255,255,255,0.2)] ring-1 ring-accent/20 hover:!bg-accent/50 hover:scale-[1.02]'
                                }`}
                        >
                            {activeAutomationIds.has(dropsCampaign.id) ? (
                                <>
                                    <Pickaxe size={14} className="animate-pulse" />
                                    <span>Automation</span>
                                </>
                            ) : (
                                <>
                                    <Pickaxe size={14} />
                                    <span>Collect Drops</span>
                                </>
                            )}
                        </button>
                        </Tooltip>
                    )}
                </div>
                <div className="p-2">
                    <Tooltip content={game.name} side="bottom"><h3 className="text-textPrimary font-medium text-sm line-clamp-2 group-hover:text-accent transition-colors">
                        {game.name}
                    </h3></Tooltip>
                    {game.viewer_count !== undefined && (
                        <p className="text-textSecondary text-xs mt-0.5">
                            {game.viewer_count.toLocaleString()} viewers
                        </p>
                    )}
                </div>
            </div>
        );
    };

    const hasCategoryDrops = activeTab === 'category' && selectedCategory && (!!(
        (selectedCategory.id && dropsGameIds.has(selectedCategory.id)) ||
        (selectedCategory.name && dropsGameNames.has(selectedCategory.name.toLowerCase()))
    ));

    const renderClipCard = (clip: TwitchClip) => (
        <MediaCard
            key={clip.id}
            kind="clip"
            title={clip.title}
            thumbnailUrl={clip.thumbnail_url || VOD_FALLBACK_THUMB}
            durationLabel={`${clip.duration.toFixed(1)}s`}
            viewCount={clip.view_count}
            channel={{ name: clip.broadcaster_name }}
            trailing={formatCardDate(clip.created_at)}
            category={clip.game_name || selectedCategory?.name}
            note={`Clipped by ${clip.creator_name}`}
            className={isOverlayMode ? '!bg-black/40 !border-white/5' : ''}
            onClick={() => {
                setHomeCategoryTab('clips');
                playMedia('clip', clip.url, { ...clip, clip_source: clipSourceOf(clip) });
            }}
        />
    );

    const renderVideoCard = (video: TwitchVideo) => (
        <MediaCard
            key={video.id}
            kind={mediaKindOfVideo(video.type)}
            title={video.title}
            thumbnailUrl={vodThumbUrl(video.thumbnail_url)}
            durationLabel={videoDurationLabel(video)}
            viewCount={video.view_count}
            progress={video.progress}
            lengthSeconds={video.length_seconds}
            status={video.status}
            channel={{ name: video.user_name }}
            trailing={formatCardDate(video.created_at)}
            category={video.game_name || selectedCategory?.name}
            className={isOverlayMode ? '!bg-black/40 !border-white/5' : ''}
            onClick={() => {
                setHomeCategoryTab('videos');
                playMedia('video', video.url, video);
            }}
        />
    );

    return (
        <div className="flex flex-col h-full">
            {/* Global SVG Definitions for Liquid Glass Heart */}
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

            {/* Top Navigation Frame - Always Center Navigation */}
            {!(activeTab === 'category' && selectedCategory) && (
                <div className="flex flex-col relative box-border overflow-hidden z-20">
                    <div className="flex gap-3 relative z-30 px-4 py-2.5 min-h-[48px] items-center justify-center border-b border-borderSubtle bg-background/95 backdrop-blur-md">
                    <div ref={searchBarRef} className="relative flex items-center glass-panel px-1.5 py-1 !rounded-xl">
                        {/* Navigation buttons - fade out when search is expanded */}
                        <LayoutGroup>
                        <div className={`flex items-center gap-1 transition-opacity duration-300 ${isSearchExpanded ? 'opacity-0' : 'opacity-100'}`}>
                            {/* The platform switcher used to sit here. It moved to
                                the title bar (PlatformSwitcher): this toolbar unmounts
                                on a category drill-down and is gone entirely while
                                watching, so it could never answer "which platform
                                am I in" for more than one screen. */}
                            <MultiNookToggle />
                            <MultiChatButton />
                            <div className="border-l border-borderSubtle h-5 mx-0.5" />
                            {isAuthenticated && (
                                <button
                                    onClick={() => { setActiveTab('following'); setIsSearchExpanded(false); }}
                                    className={`group relative px-3 py-1.5 text-sm font-medium rounded-lg transition-all duration-300 whitespace-nowrap ${activeTab === 'following'
                                        ? 'text-textPrimary'
                                        : 'text-textSecondary hover:text-textPrimary'
                                        }`}
                                >
                                    {activeTab === 'following' && (
                                        <motion.div
                                            layoutId="homeTabHighlight"
                                            className="absolute inset-0 glass-button-static rounded-lg"
                                            transition={{ type: "spring", stiffness: 350, damping: 30 }}
                                        />
                                    )}
                                    <span className={`relative z-10 flex items-center transition-all duration-300 ${activeTab !== 'following' ? 'group-hover:drop-shadow-[0_2px_4px_rgba(0,0,0,0.8)]' : ''}`}>
                                        Following
                                        {/* Counts what this tab will actually show, which
                                            changes with the platform — the Twitch number
                                            beside a Kick list was just wrong. */}
                                        {followingCount > 0 && (
                                            <span className="ml-1.5 text-xs opacity-80">
                                                {followingCount}
                                            </span>
                                        )}
                                    </span>
                                </button>
                            )}
                            <button
                                onClick={() => { setActiveTab('recommended'); setIsSearchExpanded(false); }}
                                className={`group relative px-3 py-1.5 text-sm font-medium rounded-lg transition-all duration-300 whitespace-nowrap ${activeTab === 'recommended'
                                    ? 'text-textPrimary'
                                    : 'text-textSecondary hover:text-textPrimary'
                                    }`}
                            >
                                {activeTab === 'recommended' && (
                                    <motion.div
                                        layoutId="homeTabHighlight"
                                        className="absolute inset-0 glass-button-static rounded-lg"
                                        transition={{ type: "spring", stiffness: 350, damping: 30 }}
                                    />
                                )}
                                <span className={`relative z-10 inline-block transition-all duration-300 ${activeTab !== 'recommended' ? 'group-hover:drop-shadow-[0_2px_4px_rgba(0,0,0,0.8)]' : ''}`}>Discover</span>
                            </button>
                            <button
                                onClick={handleBrowseClick}
                                className={`group relative px-3 py-1.5 text-sm font-medium rounded-lg transition-all duration-300 whitespace-nowrap ${activeTab === 'browse'
                                    ? 'text-textPrimary'
                                    : 'text-textSecondary hover:text-textPrimary'
                                    }`}
                            >
                                {activeTab === 'browse' && (
                                    <motion.div
                                        layoutId="homeTabHighlight"
                                        className="absolute inset-0 glass-button-static rounded-lg"
                                        transition={{ type: "spring", stiffness: 350, damping: 30 }}
                                    />
                                )}
                                <span className={`relative z-10 inline-block transition-all duration-300 ${activeTab !== 'browse' ? 'group-hover:drop-shadow-[0_2px_4px_rgba(0,0,0,0.8)]' : ''}`}>Categories</span>
                            </button>
                            {(searchResults.length > 0 || categorySearchResults.length > 0) && (
                                <button
                                    onClick={() => setActiveTab('search')}
                                    className={`group relative px-3 py-1.5 text-sm font-medium rounded-lg transition-all duration-300 whitespace-nowrap ${activeTab === 'search'
                                        ? 'text-textPrimary'
                                        : 'text-textSecondary hover:text-textPrimary'
                                        }`}
                                >
                                    {activeTab === 'search' && (
                                        <motion.div
                                            layoutId="homeTabHighlight"
                                            className="absolute inset-0 glass-button-static rounded-lg"
                                            transition={{ type: "spring", stiffness: 350, damping: 30 }}
                                        />
                                    )}
                                    <span className={`relative z-10 flex items-center transition-all duration-300 ${activeTab !== 'search' ? 'group-hover:drop-shadow-[0_2px_4px_rgba(0,0,0,0.8)]' : ''}`}>
                                        Results
                                        <span className="ml-1 text-xs opacity-80">{searchMode === 'categories' ? categorySearchResults.length : searchResults.length}</span>
                                    </span>
                                </button>
                            )}
                            <div className="border-l border-borderSubtle h-6 ml-1" />
                            {/* Search button - opens search */}
                            <Tooltip content="Search channels" side="top">
                            <button
                                onClick={() => setIsSearchExpanded(true)}
                                className="p-1.5 text-textSecondary hover:text-textPrimary rounded-lg transition-all ml-0.5"
                            >
                                <Search size={16} />
                            </button>
                            </Tooltip>
                        </div>
                        </LayoutGroup>

                        {/* Search overlay - expands from right to cover buttons */}
                        <motion.div
                            initial={false}
                            animate={{ 
                                clipPath: isSearchExpanded 
                                    ? 'inset(0% 0% 0% 0% round 12px)' 
                                    : 'inset(0% 0% 0% 100% round 12px)',
                                opacity: isSearchExpanded ? 1 : 0
                            }}
                            transition={{ type: "spring", stiffness: 350, damping: 30 }}
                            className="absolute -inset-[1px] flex items-center glass-input !rounded-xl z-20"
                            style={{ pointerEvents: isSearchExpanded ? 'auto' : 'none' }}
                        >
                            <input
                                ref={searchInputRef}
                                type="text"
                                value={searchQuery}
                                onChange={(e) => setSearchQuery(e.target.value)}
                                onKeyDown={handleSearchKeyPress}
                                onFocus={() => setHistoryOpen(true)}
                                onBlur={() => {
                                    // Item clicks use onMouseDown + preventDefault, so blur only
                                    // fires when focus really leaves the search — safe to close.
                                    setHistoryOpen(false);
                                    if (!searchQuery.trim()) {
                                        setIsSearchExpanded(false);
                                    }
                                }}
                                className="flex-1 bg-transparent text-center text-white text-sm px-10 py-1.5 focus:outline-none h-full w-full"
                            />

                            {/* Close button - ONLY visible when text exists */}
                            <AnimatePresence>
                            {searchQuery.trim() && (
                                <motion.div
                                    initial={{ opacity: 0, scale: 0.8 }}
                                    animate={{ opacity: 1, scale: 1 }}
                                    exit={{ opacity: 0, scale: 0.8 }}
                                    transition={{ duration: 0.15 }}
                                    className="absolute right-1.5"
                                >
                                    <Tooltip content="Close Search" side="top">
                                    <button
                                        onClick={() => {
                                            setIsSearchExpanded(false);
                                            setSearchQuery('');
                                            setSearchResults([]);
                                            setCategorySearchResults([]);
                                            if (activeTab === 'search') {
                                                setActiveTab(isAuthenticated ? 'following' : 'recommended');
                                            }
                                        }}
                                        disabled={isSearching}
                                        className={`p-1.5 rounded-full transition-all text-white/60 hover:text-textPrimary hover:bg-white/10 ${isSearching ? 'opacity-50 cursor-not-allowed' : 'cursor-pointer'}`}
                                    >
                                        <X size={16} />
                                    </button>
                                    </Tooltip>
                                </motion.div>
                            )}
                            </AnimatePresence>
                        </motion.div>

                        {/* Recent searches — shown under the bar while it's focused. Filters
                            live as you type; each entry replays its own streamer/category mode.
                            Portaled to <body> (the nav frame clips overflow). No AnimatePresence
                            wrapper: it filters out portal nodes (not valid elements), so wrapping
                            one renders nothing — the motion.div still animates in on its own. */}
                        {isSearchExpanded && historyOpen && historyRect && (() => {
                            const ql = searchQuery.trim().toLowerCase();
                            const items = recentSearches.filter((r) => !ql || r.query.toLowerCase().includes(ql));
                            if (items.length === 0) return null;
                            return createPortal(
                                <motion.div
                                    initial={{ opacity: 0, y: -4 }}
                                    animate={{ opacity: 1, y: 0 }}
                                    exit={{ opacity: 0, y: -4 }}
                                    transition={{ duration: 0.12 }}
                                    style={{ position: 'fixed', top: historyRect.top, left: historyRect.left, width: historyRect.width }}
                                    className="z-[1000] rounded-xl overflow-hidden py-1 shadow-xl border border-borderSubtle bg-background/95 backdrop-blur-md"
                                >
                                    <div className="flex items-center justify-between px-3 py-1">
                                        <span className="flex items-center gap-1.5 text-[11px] font-medium uppercase tracking-wide text-textSecondary">
                                            <Clock size={11} /> Recent · {scopeLabel}
                                        </span>
                                        <button
                                            onMouseDown={(e) => { e.preventDefault(); setRecentSearches(clearRecentSearches(searchScope)); }}
                                            className="text-[11px] text-textSecondary hover:text-textPrimary transition-colors"
                                        >
                                            Clear
                                        </button>
                                    </div>
                                    {items.map((r) => (
                                        <div
                                            key={`${r.mode}:${r.query}`}
                                            className="group mx-1 flex items-center gap-2 rounded-lg px-2 hover:bg-white/5"
                                        >
                                            <button
                                                onMouseDown={(e) => { e.preventDefault(); handleSearch({ query: r.query, mode: r.mode }); }}
                                                className="flex min-w-0 flex-1 items-center gap-2 py-1.5 text-left text-sm text-textPrimary"
                                            >
                                                {r.mode === 'categories'
                                                    ? <LayoutGrid size={14} className="flex-shrink-0 text-textSecondary" />
                                                    : <User size={14} className="flex-shrink-0 text-textSecondary" />}
                                                <span className="truncate">{r.query}</span>
                                                <span className="ml-auto flex-shrink-0 text-[11px] text-textSecondary">
                                                    {r.mode === 'categories' ? 'Category' : 'Channel'}
                                                </span>
                                            </button>
                                            <button
                                                onMouseDown={(e) => { e.preventDefault(); setRecentSearches(removeRecentSearch(searchScope, r.query, r.mode)); }}
                                                className="flex-shrink-0 rounded p-1 text-textSecondary opacity-0 transition-all hover:text-textPrimary group-hover:opacity-100"
                                                aria-label={`Remove ${r.query}`}
                                            >
                                                <X size={12} />
                                            </button>
                                        </div>
                                    ))}
                                </motion.div>,
                                document.body,
                            );
                        })()}
                    </div>
                {/* Returning to the stream is the title bar's Home/Return
                    toggle now, so it stays in one place instead of jumping
                    across the window depending on which view you are in. */}
            </div>

        </div>
        )}

            {/* Content */}
            <div ref={scrollContainerRef} className="flex-1 overflow-y-auto p-4 scrollbar-thin relative">
                
                {/* FLOATING GLASS PILL HEADER (Only in Category View) */}
                {activeTab === 'category' && selectedCategory && (
                    <div className="sticky top-4 mt-2 z-30 h-0 overflow-visible flex items-center justify-between w-full pointer-events-none">
                        <div className="flex items-center gap-3 text-textPrimary">
                            {/* Back Button */}
                            <Tooltip content="Back to Browse" side="right">
                                <button
                                    onClick={handleBackToBrowse}
                                    className="h-[44px] w-[44px] glass-panel hover:bg-glass-hover rounded-xl flex items-center justify-center shadow-lg pointer-events-auto bg-background/80 backdrop-blur-md transition-colors"
                                >
                                    <ArrowLeft size={20} className="text-textSecondary hover:text-textPrimary transition-colors" />
                                </button>
                            </Tooltip>

                            {/* Tiny Category Pill - Dropping in playfully */}
                            <div
                                style={{
                                    opacity: isScrolledPastHero ? 1 : 0,
                                    transform: `translateY(${isScrolledPastHero ? 0 : 10}px)`,
                                    transition: 'opacity 0.2s ease, transform 0.2s ease',
                                }}
                                className="h-[44px] glass-panel rounded-xl px-4 flex items-center shadow-lg pointer-events-auto bg-background/80 backdrop-blur-md border border-white/5"
                            >
                                <span className="font-bold text-sm truncate max-w-[200px] sm:max-w-[400px]">
                                    {selectedCategory.name}
                                </span>
                            </div>
                        </div>
                        
                    </div>
                )}
                
                {/* NATURAL SCROLLING HERO BANNER */}
                {activeTab === 'category' && selectedCategory && (
                    <>
                    <div ref={heroSentinelRef} className="h-px w-full -mt-px pointer-events-none" aria-hidden="true" />
                    <div 
                        style={{ opacity: isScrolledPastHero ? 0 : 1, transition: 'opacity 0.15s ease' }}
                        className="flex gap-4 sm:gap-6 w-full max-w-[900px] items-start pb-6 mt-2 ml-[56px] relative z-10"
                    >
                        {/* Hero Box Art */}
                        <div className="flex-shrink-0">
                            <div className={`w-[108px] h-[144px] sm:w-[144px] sm:h-[192px] bg-white/5 rounded-xl overflow-hidden shadow-[0_8px_30px_rgba(0,0,0,0.6)] border border-white/10 flex items-center justify-center ${isLoadingCategoryDetails && !categoryDetails?.boxArtUrl && !selectedCategory?.box_art_url ? 'animate-pulse' : ''}`}>
                                {(categoryDetails?.boxArtUrl || selectedCategory?.box_art_url) ? (
                                    <img 
                                        src={getGameBoxArt(categoryDetails?.boxArtUrl || selectedCategory.box_art_url)} 
                                        alt={selectedCategory.name}
                                        className={`w-full h-full object-cover transition-opacity duration-300 ${isLoadingCategoryDetails && !categoryDetails?.boxArtUrl ? 'opacity-50' : 'opacity-100'}`}
                                        loading="lazy"
                                    />
                                ) : (
                                    <LayoutGrid size={32} className="text-white/20" />
                                )}
                            </div>
                        </div>
                        
                        {/* Category Metadata Column */}
                        <div className="flex flex-col flex-1 min-w-0 pr-16 mt-1 sm:mt-2 max-w-[600px]">
                            {/* Title & Followers */}
                            <div className="flex items-center gap-3 mb-2 flex-wrap">
                                <h1 className="text-2xl sm:text-3xl font-black text-transparent bg-clip-text bg-gradient-to-r from-textPrimary to-textSecondary truncate leading-tight">
                                    {categoryDetails?.displayName || selectedCategory.name}
                                </h1>
                               {categoryDetails?.followersCount != null && (
                                    <div className="glass-badge flex items-center gap-1.5 whitespace-nowrap !bg-white/5 backdrop-blur-md px-2.5 py-1 shrink-0">
                                        <Users size={12} className="text-accent" />
                                        <span className="text-[11px] font-bold text-textPrimary tracking-wide">
                                            {new Intl.NumberFormat('en-US', { notation: "compact", compactDisplay: "short" }).format(categoryDetails.followersCount)} Followers
                                        </span>
                                    </div>
                                )}
                            </div>
                            
                            {/* Tags */}
                            <div className="flex flex-wrap gap-1.5 mb-3">
                                {hasCategoryDrops && (
                                    <button 
                                        onClick={(e) => {
                                            e.stopPropagation();
                                            if (selectedCategory?.name) {
                                                openDropsWithSearch(selectedCategory.name);
                                            }
                                        }}
                                        className="drops-badge-glass hover:brightness-125 hover:scale-105 active:scale-95 transition-all cursor-pointer !text-[10px] !px-2 !py-0.5 mr-1"
                                    >
                                        <Gift size={11} />
                                        <span>DROPS ENABLED</span>
                                    </button>
                                )}
                                {isLoadingCategoryDetails ? (
                                    <>
                                        <div className="h-5 w-16 bg-white/5 rounded-full animate-pulse px-2"></div>
                                        <div className="h-5 w-20 bg-white/5 rounded-full animate-pulse px-2"></div>
                                        <div className="h-5 w-14 bg-white/5 rounded-full animate-pulse px-2"></div>
                                    </>
                                ) : categoryDetails?.tags && categoryDetails.tags.length > 0 ? (
                                    categoryDetails.tags.slice(0, 5).map(tag => (
                                        <span key={tag.id} className="text-[10px] font-semibold px-2 py-0.5 rounded-full bg-white/5 border border-white/10 text-textSecondary truncate max-w-[150px] shadow-sm tracking-wide">
                                            {tag.localizedName}
                                        </span>
                                    ))
                                ) : null}
                            </div>
                            
                            {/* Description Accordion */}
                            <div className="relative group">
                                {isLoadingCategoryDetails ? (
                                     <div className="space-y-2 mt-1 w-full max-w-[400px]">
                                         <div className="h-3 bg-white/10 rounded w-full animate-pulse"></div>
                                         <div className="h-3 bg-white/10 rounded w-5/6 animate-pulse"></div>
                                     </div>
                                ) : categoryDetails?.description ? (
                                    <div className="relative">
                                        <motion.div 
                                            initial={false}
                                            animate={{ height: isDescriptionExpanded ? "auto" : 48 }}
                                            transition={{ duration: 0.25, ease: "easeOut" }}
                                            onAnimationComplete={() => {
                                                if (!isDescriptionExpanded) {
                                                    setIsDescriptionClamped(true);
                                                }
                                            }}
                                            className="overflow-hidden"
                                        >
                                            <p className={`text-[13px] sm:text-[14px] text-textSecondary/90 font-medium leading-[24px] ${isDescriptionClamped ? 'line-clamp-2' : ''}`}>
                                                {categoryDetails.description}
                                            </p>
                                        </motion.div>
                                        {!isDescriptionExpanded && categoryDetails.description.length > 100 && (
                                            <div className="mt-1 flex items-center justify-start pointer-events-none">
                                                <button 
                                                    onClick={(e) => { 
                                                        e.stopPropagation(); 
                                                        setIsDescriptionClamped(false);
                                                        setIsDescriptionExpanded(true); 
                                                    }}
                                                    className="text-[12px] font-bold text-accent hover:text-textPrimary pointer-events-auto transition-colors"
                                                >
                                                    Read More
                                                </button>
                                            </div>
                                        )}
                                        {isDescriptionExpanded && (
                                            <div className="mt-1">
                                                <button 
                                                    onClick={(e) => { e.stopPropagation(); setIsDescriptionExpanded(false); }}
                                                    className="text-[11px] font-bold text-textSecondary hover:text-textPrimary transition-colors"
                                                >
                                                    Show Less
                                                </button>
                                            </div>
                                        )}
                                    </div>
                                ) : (
                                    <p className="text-[13px] text-textSecondary/40 italic">No description available</p>
                                )}
                            </div>
                        </div>
                    </div>
                    </>
                )}

                {/* Only show full LoadingWidget during initial app load, not user-initiated login */}
                {isLoading && !isAuthenticated && !hasInitialized && (
                    <LoadingWidget useFunnyMessages={true} />
                )}

                {/* Platform categories — the same tile grid Twitch gets, so the
                    Categories tab means the same thing on every platform.
                    Picking one drills into its live streams below. */}
                {showsProviderCategories && !isLoadingProvider && providerCategories.length > 0 && (
                    <div className="grid grid-cols-3 sm:grid-cols-4 md:grid-cols-5 lg:grid-cols-6 xl:grid-cols-7 2xl:grid-cols-8 gap-3">
                        {providerCategories.map((cat) => (
                            <div
                                key={cat.id || cat.name}
                                className={`glass-panel media-card cursor-pointer hover:bg-glass-hover transition-all duration-200 group overflow-hidden relative ${isOverlayMode ? '!bg-black/40 !border-white/5' : ''}`}
                                onClick={() => setProviderCategory(cat)}
                            >
                                <div className="relative overflow-hidden">
                                    <img
                                        loading="lazy"
                                        src={cat.thumbnail}
                                        alt={cat.name}
                                        className="w-full aspect-[3/4] object-cover group-hover:scale-105 transition-transform duration-200"
                                        onError={(e) => { (e.target as HTMLImageElement).style.visibility = 'hidden'; }}
                                    />
                                </div>
                                <div className="p-2">
                                    <h3 className="text-textPrimary font-medium text-sm line-clamp-1 group-hover:text-accent transition-colors">
                                        {cat.name}
                                    </h3>
                                    <p className="text-textSecondary text-xs mt-0.5">
                                        {formatViewerCount(cat.viewer_count)} viewers
                                    </p>
                                </div>
                            </div>
                        ))}
                    </div>
                )}

                {/* Drill-down header: which category the stream grid below is showing. */}
                {isProviderView && activeTab === 'browse' && providerCategory && (
                    <div className="mb-3 flex items-center gap-2">
                        <button
                            onClick={() => setProviderCategory(null)}
                            className="glass-button-secondary px-2.5 py-1.5 text-[13px] text-textSecondary hover:text-textPrimary rounded-lg transition-colors"
                        >
                            ← All categories
                        </button>
                        <h2 className="text-textPrimary font-semibold text-[15px]">{providerCategory.name}</h2>
                        <span className="text-textSecondary text-xs">
                            {providerCategory.viewer_count.toLocaleString()} viewers
                        </span>
                    </div>
                )}

                {/* Progressive Scroll Category Profile is now perfectly enclosed within the Top Navigation Frame! */}
                {/* Browse View - Game Categories. Twitch-only: the category grid
                    is built from Helix top-games. With a platform filter active
                    the platform's own category tiles render above instead. */}
                {activeTab === 'browse' && !isProviderView && (
                    <>
                        {isLoadingGames ? (
                            <div className="relative h-full min-h-[400px] flex items-center justify-center">
                                <LoadingWidget useFunnyMessages={false} message="Loading categories..." fullScreen={false} />
                            </div>
                        ) : topGames.length === 0 ? (
                            <div className="flex items-center justify-center h-full">
                                <div className="text-center glass-panel p-6 max-w-sm">
                                    <h3 className="text-base font-bold text-textPrimary mb-1">No Categories Found</h3>
                                    <p className="text-textSecondary text-sm">Could not load categories.</p>
                                </div>
                            </div>
                        ) : (
                            <>
                                <div className="grid grid-cols-3 sm:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 2xl:grid-cols-8 gap-3">
                                    {topGames.map(game => renderCategoryCard(game))}
                                </div>
                                {/* Unified view: the other platforms' categories,
                                    kept in their own labelled row rather than
                                    interleaved — a Twitch game id and a Kick
                                    category id are different taxonomies, and
                                    mixing them would make the grid lie about
                                    which one a tile belongs to. */}
                                {isUnifiedView && unifiedCategories.length > 0 && (
                                    <div className="mt-6 border-t border-borderSubtle/40 pt-4">
                                        <h3 className="mb-3 text-sm font-semibold text-textSecondary">
                                            On other platforms
                                        </h3>
                                        <div className="grid grid-cols-3 sm:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 2xl:grid-cols-8 gap-3">
                                            {unifiedCategories.map((cat) => (
                                                <div
                                                    key={`${cat.provider}:${cat.id || cat.name}`}
                                                    className={`glass-panel media-card cursor-pointer hover:bg-glass-hover transition-all duration-200 group overflow-hidden relative ${isOverlayMode ? '!bg-black/40 !border-white/5' : ''}`}
                                                    onClick={() => {
                                                        // Jump into that platform's context, then open the category.
                                                        useAppStore.getState().setActivePlatform(cat.provider);
                                                        setProviderCategory(cat);
                                                        setActiveTab('browse');
                                                    }}
                                                >
                                                    <div className="relative overflow-hidden">
                                                        <img
                                                            loading="lazy"
                                                            src={cat.thumbnail}
                                                            alt={cat.name}
                                                            className="w-full aspect-[3/4] object-cover group-hover:scale-105 transition-transform duration-200"
                                                            onError={(e) => { (e.target as HTMLImageElement).style.visibility = 'hidden'; }}
                                                        />
                                                    </div>
                                                    <div className="p-2">
                                                        <h3 className="text-textPrimary font-medium text-sm line-clamp-1 group-hover:text-accent transition-colors">{cat.name}</h3>
                                                        {/* Same rule as the stream cards: a bare mark in the
                                                            tile's bottom-right, never over the artwork. */}
                                                        <div className="mt-0.5 flex items-center justify-between gap-1">
                                                            <span className="text-textSecondary text-xs truncate">{formatViewerCount(cat.viewer_count)} viewers</span>
                                                            <span className="flex flex-shrink-0 items-center opacity-80">
                                                                <ProviderLogo provider={cat.provider} size={12} />
                                                            </span>
                                                        </div>
                                                    </div>
                                                </div>
                                            ))}
                                        </div>
                                    </div>
                                )}
                                {/* Loading indicator for infinite scroll */}
                                {isLoadingMoreGames && (
                                    <div className="flex justify-center items-center py-6">
                                        <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-accent"></div>
                                    </div>
                                )}
                                {/* End of categories message */}
                                {!hasMoreGames && topGames.length > 0 && (
                                    <div className="text-center py-6">
                                        <p className="text-textSecondary text-xs">No more categories</p>
                                    </div>
                                )}
                            </>
                        )}
                    </>
                )}

                {/* Category Streams View */}
                {activeTab === 'category' && (
                    <div className="pt-2">
                        {/* Category Sub-Navigation Tabs and Toolbar */}
                        <div className="flex items-center gap-4 mb-4 border-b border-white/5 pb-3 w-full mt-2">
                            <div className="flex gap-2">
                                <button
                                    onClick={() => setCategoryActiveTab('live')}
                                    className={`px-4 py-1.5 text-sm font-bold !rounded-lg transition-all relative ${categoryActiveTab === 'live' ? 'glass-input text-textPrimary' : 'glass-button text-textSecondary/80 hover:text-textPrimary'}`}
                                >
                                    Live
                                </button>
                                <button
                                    onClick={() => setCategoryActiveTab('clips')}
                                    className={`px-4 py-1.5 text-sm font-bold !rounded-lg transition-all relative ${categoryActiveTab === 'clips' ? 'glass-input text-textPrimary' : 'glass-button text-textSecondary/80 hover:text-textPrimary'}`}
                                >
                                    Clips
                                </button>
                                <button
                                    onClick={() => setCategoryActiveTab('videos')}
                                    className={`px-4 py-1.5 text-sm font-bold !rounded-lg transition-all relative ${categoryActiveTab === 'videos' ? 'glass-input text-textPrimary' : 'glass-button text-textSecondary/80 hover:text-textPrimary'}`}
                                >
                                    Videos
                                </button>
                            </div>

                            {/* Filter/Search Toolbar for Live streams: one all-in-one box that
                                filters streams by tag/title/streamer and drops down matching
                                tag suggestions as you type. */}
                            {categoryActiveTab === 'live' && (
                                <div className="flex-1 flex justify-center">
                                    <CategorySearchBox
                                        value={categoryLiveDraft}
                                        onChange={setCategoryLiveDraft}
                                        tagOptions={availableCategoryTags}
                                        tagSuggestions={categoryStreamTags}
                                        selectedTags={selectedCategoryTags}
                                        onToggleTag={toggleCategoryTag}
                                    />
                                </div>
                            )}

                            {/* Filter/Search Toolbar for Clips and Videos */}
                            {(categoryActiveTab === 'clips' || categoryActiveTab === 'videos') && (
                                <div className="flex items-center gap-2 ml-auto">
                                    {/* Search Box */}
                                    <div className="relative group">
                                        <div className="absolute inset-y-0 left-0 pl-3 flex items-center pointer-events-none text-textSecondary group-focus-within:text-accent transition-colors">
                                            <Search size={14} />
                                        </div>
                                        <input
                                            type="text"
                                            placeholder={`Search ${categoryActiveTab}...`}
                                            value={mediaSearchQuery}
                                            onChange={(e) => setMediaSearchQuery(e.target.value)}
                                            className="glass-input !rounded-lg pl-9 pr-3 py-1.5 w-[140px] focus:w-[220px] outline-none text-sm text-textPrimary placeholder-textSecondary/50 font-medium transition-all"
                                        />
                                        {mediaSearchQuery && (
                                            <button 
                                                onClick={() => setMediaSearchQuery('')}
                                                className="absolute inset-y-0 right-0 pr-3 flex items-center text-textSecondary hover:text-accent transition-colors"
                                            >
                                                <X size={14} />
                                            </button>
                                        )}
                                    </div>
                                    <div className="h-5 w-px bg-white/10 mx-1"></div>
                                    {/* Dropdowns */}
                                    {categoryActiveTab === 'clips' && (
                                        <GlassSelect
                                            value={clipsPeriod}
                                            onChange={(val) => setClipsPeriod(val)}
                                            options={[
                                                { value: '24h', label: 'Last 24 Hours' },
                                                { value: '7d', label: 'Last 7 Days' },
                                                { value: '30d', label: 'Last 30 Days' },
                                                { value: 'all', label: 'All Time' }
                                            ]}
                                        />
                                    )}
                                    {categoryActiveTab === 'videos' && (
                                        <>
                                            <GlassSelect
                                                value={videosSort}
                                                onChange={(val) => setVideosSort(val)}
                                                options={[
                                                    { value: 'time', label: 'Recent' },
                                                    { value: 'trending', label: 'Trending' },
                                                    { value: 'views', label: 'Most Viewed' }
                                                ]}
                                            />
                                            <GlassSelect
                                                value={videosPeriod}
                                                onChange={(val) => setVideosPeriod(val)}
                                                options={[
                                                    { value: 'all', label: 'All Time' },
                                                    { value: 'day', label: 'Last 24 Hours' },
                                                    { value: 'week', label: 'Last 7 Days' },
                                                    { value: 'month', label: 'Last 30 Days' }
                                                ]}
                                            />
                                        </>
                                    )}
                                </div>
                            )}
                        </div>

                        {/* Active tag filter chips */}
                        {categoryActiveTab === 'live' && selectedCategoryTags.length > 0 && (
                            <div className="flex flex-wrap items-center gap-1.5 mb-4 -mt-1">
                                {selectedCategoryTags.map(tag => (
                                    <button
                                        key={tag}
                                        onClick={() => toggleCategoryTag(tag)}
                                        className="group flex items-center gap-1 text-[11px] font-semibold pl-2.5 pr-1.5 py-1 rounded-md bg-accent/20 text-accent hover:bg-accent/30 transition-colors"
                                    >
                                        <span className="truncate max-w-[160px]">{tag}</span>
                                        <X size={12} className="opacity-70 group-hover:opacity-100" />
                                    </button>
                                ))}
                                <button
                                    onClick={() => setSelectedCategoryTags([])}
                                    className="text-[11px] font-semibold px-2 py-1 text-textSecondary hover:text-accent transition-colors"
                                >
                                    Clear all
                                </button>
                            </div>
                        )}

                        {/* Rendering Logic based on categoryActiveTab */}
                        {categoryActiveTab === 'live' && (
                            <>
                                {(tagMode ? isLoadingTagStreams : isLoadingCategoryStreams) ? (
                                    <div className="flex items-center justify-center h-full">
                                        <div className="text-center">
                                            <div className="animate-spin rounded-full h-12 w-12 border-4 border-glass border-t-accent mx-auto mb-3" />
                                            <p className="text-textSecondary text-xs">Loading streams...</p>
                                        </div>
                                    </div>
                                ) : baseLiveStreams.length === 0 ? (
                                    <div className="flex items-center justify-center h-full">
                                        <div className="text-center glass-panel p-6 max-w-sm">
                                            <h3 className="text-base font-bold text-textPrimary mb-1">No Live Streams</h3>
                                            <p className="text-textSecondary text-sm">
                                                {tagMode
                                                    ? `No live streams in ${selectedCategory?.name} match ${selectedCategoryTags.length === 1 ? 'that tag' : 'those tags'}.`
                                                    : `No one is streaming ${selectedCategory?.name}.`}
                                            </p>
                                        </div>
                                    </div>
                                ) : (
                                    <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 2xl:grid-cols-5 gap-3">
                                        {baseLiveStreams.map(stream => {
                                            // Drops indicator is elevated to the hero banner in Category view!
                                            // We explicitly disable drops badging on individual stream cards here to reduce noise.
                                            const hasDrops = false;
                                            const favoriteId = favoriteIdOf(stream);
                                            const isFavorite = !!favoriteId && isFavoriteStreamer(favoriteId);

                                            return (() => {
                                                const isQueued = isInMultiNook(stream.user_login, streamProvider(stream));
                                                const isSuckingUp = suckUpKey === makeKey(streamProvider(stream), stream.user_login);
                                                const isMaterializing = materializingKey === makeKey(streamProvider(stream), stream.user_login);

                                                return (
                                                    <motion.div
                                                        layout
                                                        transition={{ type: "spring", stiffness: 350, damping: 25 }}
                                                        key={stream.id}
                                                        data-avatar-key={streamKey(stream)}
                                                        className={`p-2.5 transition-all duration-200 group relative ${
                                                            isQueued && !isSuckingUp
                                                                ? 'ghost-card rounded-lg cursor-default'
                                                                : isQueued && isSuckingUp
                                                                    ? `glass-panel media-card cursor-default ${isOverlayMode ? '!bg-black/40 !border-white/5' : ''}`
                                                                    : `glass-panel media-card cursor-pointer hover:bg-glass-hover ${isOverlayMode ? '!bg-black/40 !border-white/5' : ''} ${hasDrops ? 'ring-2 ring-accent/60' : ''}`
                                                        }`}
                                                        style={!isQueued && hasDrops ? { boxShadow: '0 0 12px var(--color-accent-muted)' } : undefined}
                                                        onClick={(e) => !isQueued && handleStreamClick(e, stream)}
                                                        onContextMenu={(e) => !isQueued && useContextMenuStore.getState().openMenu(e, stream)}
                                                    >
                                                        {isQueued && !isSuckingUp ? (
                                                            /* Ghost state — recall button + label */
                                                            <>
                                                                <div className="invisible">
                                                                    <div className="relative mb-2 overflow-hidden rounded aspect-video" />
                                                                    <div className="space-y-0.5">
                                                                        <div className="h-4" />
                                                                        <div className="h-3" />
                                                                    </div>
                                                                </div>
                                                                <div className="absolute inset-0 flex flex-col items-center justify-center gap-1.5 animate-ghost-label">
                                                                    <LayoutGrid size={18} className="text-accent/50" />
                                                                    <span className="text-accent text-xs font-semibold truncate max-w-[80%]">{stream.user_name}</span>
                                                                    <span className="text-textSecondary text-[10px]">Queued in MultiNook</span>
                                                                    <Tooltip content="Recall from MultiNook" side="bottom">
                                                                        <button
                                                                            onClick={(e) => {
                                                                                e.stopPropagation();
                                                                                const card = (e.currentTarget as HTMLElement).closest('.ghost-card');
                                                                                const rect = card?.getBoundingClientRect();
                                                                                const cx = rect ? rect.left + rect.width / 2 : e.clientX;
                                                                                const cy = rect ? rect.top + rect.height / 2 : e.clientY;
                                                                                triggerRecallAnimation(stream.user_login, cx, cy, streamProvider(stream));
                                                                            }}
                                                                            className="glass-button !rounded-full !p-1.5 mt-1 text-textSecondary hover:text-accent transition-colors"
                                                                        >
                                                                            <Undo2 size={14} strokeWidth={2} />
                                                                        </button>
                                                                    </Tooltip>
                                                                </div>
                                                            </>
                                                        ) : (
                                                            /* Normal content, suck-up, or materialize animation */
                                                            <div className={isSuckingUp ? 'animate-multinook-suck-up' : isMaterializing ? 'animate-multinook-materialize' : undefined}>
                                                                {!isSuckingUp && <QuickAddButton stream={stream} />}
                                                                <div className="relative mb-2 overflow-hidden rounded">
                                                                    <img
                                                                        loading="lazy"
                                                                        src={getThumbnailUrl(stream.thumbnail_url)}
                                                                        alt={stream.title}
                                                                        className="w-full aspect-video object-cover group-hover:scale-105 transition-transform duration-200"
                                                                    />
                                                                    <div className="absolute top-1.5 left-1.5 flex items-center gap-1">
                                                                        <div className="live-dot text-xs px-1.5 py-0.5">LIVE</div>
                                                                        {hasDrops && (
                                                                            <div className="drops-badge-glass">
                                                                                <Gift size={10} />
                                                                                <span>DROPS</span>
                                                                            </div>
                                                                        )}
                                                                        {activeHypeTrainChannels.get(stream.user_id) && (
                                                                            <div className={activeHypeTrainChannels.get(stream.user_id)?.isGolden ? 'hype-train-badge-glass-golden' : 'hype-train-badge-glass'}>
                                                                                <svg className="w-2.5 h-2.5" viewBox="0 0 15 13" fill="none">
                                                                                    <path fillRule="evenodd" clipRule="evenodd" d="M4.10001 0.549988H2.40001V4.79999H0.700012V10.75H1.55001C1.55001 11.6889 2.31113 12.45 3.25001 12.45C4.1889 12.45 4.95001 11.6889 4.95001 10.75H5.80001C5.80001 11.6889 6.56113 12.45 7.50001 12.45C8.4389 12.45 9.20001 11.6889 9.20001 10.75H10.05C10.05 11.6889 10.8111 12.45 11.75 12.45C12.6889 12.45 13.45 11.6889 13.45 10.75H14.3V0.549988H6.65001V2.24999H7.50001V4.79999H4.10001V0.549988ZM12.6 9.04999V6.49999H2.40001V9.04999H12.6ZM9.20001 4.79999H12.6V2.24999H9.20001V4.79999Z" fill="currentColor" />
                                                                                </svg>
                                                                                <span>LVL {activeHypeTrainChannels.get(stream.user_id)?.level}</span>
                                                                            </div>
                                                                        )}
                                                                    </div>
                                                                    <div className="absolute bottom-1.5 left-1.5 px-2 py-0.5 glass-badge text-white text-[10px] font-medium rounded">
                                                                        {stream.viewer_count.toLocaleString()} viewers
                                                                    </div>
                                                                    {watchStreaks[stream.user_id] > 0 && (
                                                                        <div className="absolute bottom-1.5 right-1.5">
                                                                            <Tooltip content={`${watchStreaks[stream.user_id]} Stream Watch Streak`} side="top">
                                                                            <div className="flex items-center gap-1 font-bold text-[10px] leading-tight px-1.5 py-0.5 rounded shadow-[0_0_10px_color-mix(in_srgb,var(--color-warning)_25%,transparent)] bg-amber-500/10 text-amber-400 border border-amber-500/30 backdrop-blur-md">
                                                                                <Flame size={10} className="stroke-[2.5]" />
                                                                                <span>{watchStreaks[stream.user_id]}</span>
                                                                            </div>
                                                                            </Tooltip>
                                                                        </div>
                                                                    )}
                                                                </div>
                                                                <div className="space-y-0.5">
                                                                    <h3 className="text-textPrimary font-medium text-[13px] leading-tight line-clamp-1 group-hover:text-accent transition-colors">
                                                                        <StreamTitleWithEmojis title={stream.title} />
                                                                    </h3>
                                                                    <div className="flex items-center justify-between">
                                                                        <div className="flex items-center gap-1">
                                                                            {(stream.profile_image_url || cardAvatars[streamKey(stream)]) && (
                                                                                <img
                                                                                    loading="lazy"
                                                                                    src={stream.profile_image_url || cardAvatars[streamKey(stream)]}
                                                                                    alt=""
                                                                                    className="w-4 h-4 rounded-full object-cover flex-shrink-0 ring-1 ring-borderSubtle"
                                                                                />
                                                                            )}
                                                                            <p className="text-textSecondary text-[11px] font-medium">{stream.user_name}</p>
                                                                            {stream.broadcaster_type === 'partner' && (
                                                                                <svg className="w-3 h-3 flex-shrink-0" viewBox="0 0 16 16" fill="#9146FF">
                                                                                    <path fillRule="evenodd" d="M12.5 3.5 8 2 3.5 3.5 2 8l1.5 4.5L8 14l4.5-1.5L14 8l-1.5-4.5ZM7 11l4.5-4.5L10 5 7 8 5.5 6.5 4 8l3 3Z" clipRule="evenodd"></path>
                                                                                </svg>
                                                                            )}
                                                                        </div>
                                                                        {/* Browsing a category is one of the two places you
                                                                            actually FIND someone new, so it needs the same
                                                                            heart the main grid has. This card is a separate
                                                                            renderer, so it gets its own copy. */}
                                                                        <Tooltip content={isFavorite ? 'Remove from favorites' : 'Add to favorites'} side="top">
                                                                        <button
                                                                            onClick={(e) => { void handleFavoriteClick(e, stream); }}
                                                                            className="p-1 flex items-center justify-center bg-transparent transition-transform duration-300 hover:scale-110 active:scale-95 flex-shrink-0"
                                                                        >
                                                                            <Heart
                                                                                size={14}
                                                                                fill={isFavorite ? "url(#glass-heart-fill)" : "none"}
                                                                                stroke={isFavorite ? "url(#glass-heart-stroke)" : "currentColor"}
                                                                                strokeWidth={isFavorite ? 1.5 : 2}
                                                                                className={`transition-all duration-300 ${isFavorite ? 'drop-shadow-[0_4px_8px_color-mix(in_srgb,var(--color-highlight-pink)_50%,transparent)]' : 'text-textSecondary hover:text-textPrimary opacity-0 group-hover:opacity-100'} ${favoriteId && animatingHearts.has(favoriteId) ? 'animate-heart-break' : ''}`}
                                                                            />
                                                                        </button>
                                                                        </Tooltip>
                                                                    </div>
                                                                    {stream.tags && stream.tags.length > 0 && (
                                                                        <StreamTileTags
                                                                            tags={stream.tags}
                                                                            selectedTags={selectedCategoryTags}
                                                                            onToggleTag={toggleCategoryTag}
                                                                        />
                                                                    )}
                                                                </div>
                                                            </div>
                                                        )}
                                                    </motion.div>
                                                );
                                            })();
                                        })}
                                    </div>
                                )}
                                {/* Loading indicator for infinite scroll */}
                                {(tagMode ? isLoadingMoreTagStreams : isLoadingMoreCategoryStreams) && (
                                    <div className="flex justify-center items-center py-6 w-full">
                                        <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-accent"></div>
                                    </div>
                                )}
                                {/* End of streams message */}
                                {!(tagMode ? hasMoreTagStreams : hasMoreCategoryStreams) && baseLiveStreams.length > 0 && (
                                    <div className="text-center py-6 w-full">
                                        <p className="text-textSecondary text-xs">
                                            {tagMode ? 'No more streams with these tags' : 'No more category streams'}
                                        </p>
                                    </div>
                                )}
                            </>
                        )}
                        
                        {categoryActiveTab === 'clips' && (
                            <>
                                {isLoadingClips ? (
                                    <div className="flex items-center justify-center h-full pt-10">
                                        <div className="text-center">
                                            <div className="animate-spin rounded-full h-12 w-12 border-4 border-glass border-t-accent mx-auto mb-3" />
                                            <p className="text-textSecondary text-xs">Loading clips...</p>
                                        </div>
                                    </div>
                                ) : categoryClips.length === 0 ? (
                                    <div className="flex items-center justify-center h-[300px]">
                                        <div className="text-center glass-panel p-6 max-w-sm">
                                            <h3 className="text-base font-bold text-textPrimary mb-1">No Clips Found</h3>
                                            <p className="text-textSecondary text-sm">
                                                No clips have been generated for {selectedCategory?.name} yet.
                                            </p>
                                        </div>
                                    </div>
                                ) : (() => {
                                     const filteredClips = categoryClips.filter(c => 
                                         !mediaSearchQuery || 
                                         c.title.toLowerCase().includes(mediaSearchQuery.toLowerCase()) || 
                                         c.broadcaster_name.toLowerCase().includes(mediaSearchQuery.toLowerCase())
                                     );
                                     
                                     if (filteredClips.length === 0) {
                                         return (
                                             <div className="flex items-center justify-center h-[300px]">
                                                 <div className="text-center glass-panel p-6 max-w-sm">
                                                     <h3 className="text-base font-bold text-textPrimary mb-1">No Results Search</h3>
                                                     <p className="text-textSecondary text-sm">
                                                         No clips match your search "{mediaSearchQuery}".
                                                     </p>
                                                 </div>
                                             </div>
                                         );
                                     }

                                     return (
                                         <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 2xl:grid-cols-6 gap-3">
                                             {filteredClips.map(clip => renderClipCard(clip))}
                                         </div>
                                     );
                                 })()}
                                {/* Loading indicator for infinite scroll */}
                                {isLoadingMoreClips && (
                                    <div className="flex justify-center items-center py-6 w-full">
                                        <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-accent"></div>
                                    </div>
                                )}
                                {/* End of clips message */}
                                {!hasMoreCategoryClips && categoryClips.length > 0 && (
                                    <div className="text-center py-6 w-full">
                                        <p className="text-textSecondary text-xs">End of clips</p>
                                    </div>
                                )}
                            </>
                        )}
                        
                        {categoryActiveTab === 'videos' && (
                            <>
                                {isLoadingVideos ? (
                                    <div className="flex items-center justify-center h-full pt-10">
                                        <div className="text-center">
                                            <div className="animate-spin rounded-full h-12 w-12 border-4 border-glass border-t-accent mx-auto mb-3" />
                                            <p className="text-textSecondary text-xs">Loading videos...</p>
                                        </div>
                                    </div>
                                ) : categoryVideos.length === 0 ? (
                                    <div className="flex items-center justify-center h-[300px]">
                                        <div className="text-center glass-panel p-6 max-w-sm">
                                            <h3 className="text-base font-bold text-textPrimary mb-1">No Videos Found</h3>
                                            <p className="text-textSecondary text-sm">
                                                No videos exist for {selectedCategory?.name}.
                                            </p>
                                        </div>
                                    </div>
                                ) : (() => {
                                     const filteredVideos = categoryVideos.filter(v => 
                                         !mediaSearchQuery || 
                                         v.title.toLowerCase().includes(mediaSearchQuery.toLowerCase()) || 
                                         v.user_name.toLowerCase().includes(mediaSearchQuery.toLowerCase())
                                     );

                                     if (filteredVideos.length === 0) {
                                         return (
                                             <div className="flex items-center justify-center h-[300px]">
                                                 <div className="text-center glass-panel p-6 max-w-sm">
                                                     <h3 className="text-base font-bold text-textPrimary mb-1">No Results Search</h3>
                                                     <p className="text-textSecondary text-sm">
                                                         No videos match your search "{mediaSearchQuery}".
                                                     </p>
                                                 </div>
                                             </div>
                                         );
                                     }

                                     return (
                                         <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 2xl:grid-cols-6 gap-3">
                                             {filteredVideos.map(video => renderVideoCard(video))}
                                         </div>
                                     );
                                 })()}
                                {/* Loading indicator for infinite scroll */}
                                {isLoadingMoreVideos && (
                                    <div className="flex justify-center items-center py-6 w-full">
                                        <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-accent"></div>
                                    </div>
                                )}
                                {/* End of videos message */}
                                {!hasMoreCategoryVideos && categoryVideos.length > 0 && (
                                    <div className="text-center py-6 w-full">
                                        <p className="text-textSecondary text-xs">End of videos</p>
                                    </div>
                                )}
                            </>
                        )}
                    </div>
                )}

                {/* Following/Recommended/Search Views. With a platform filter
                    active the Categories tab shows that platform's directory
                    here too, since it has no Twitch-style category grid. */}
                {(activeTab === 'following' || activeTab === 'recommended' || activeTab === 'search' || (showsProviderDirectory && activeTab === 'browse')) && (
                    <>
                        {isSearching || isLoadingProvider ? (
                            <div className="flex items-center justify-center h-full">
                                <div className="text-center">
                                    <div className="animate-spin rounded-full h-12 w-12 border-4 border-glass border-t-accent mx-auto mb-3" />
                                    <p className="text-textSecondary text-xs">{isLoadingProvider ? 'Loading streams...' : 'Searching...'}</p>
                                </div>
                            </div>
                        ) : /* The Twitch login prompt is Twitch's own: other platforms browse signed out. */
                        !isAuthenticated && activeTab === 'following' && !isProviderView ? (
                            <div className="flex items-center justify-center h-full">
                                <div className="text-center glass-panel p-6 max-w-sm">
                                    <h3 className="text-base font-bold text-textPrimary mb-1">Not Logged In</h3>
                                    <p className="text-textSecondary text-sm mb-4">
                                        Log in to see your followed streams.
                                    </p>
                                    <div className="flex justify-center">
                                        <PlatformLoginButton
                                            provider="twitch"
                                            onClick={loginToTwitch}
                                            busy={isLoading}
                                            busyLabel="Logging in…"
                                        />
                                    </div>
                                </div>
                            </div>
                        ) : /* The same wall, for the platform you're actually looking at.
                               Following is the one tab that is meaningless signed out —
                               there is no list to show — so it asks for the account
                               instead of falling through to an anonymous grid. */
                        isProviderView && activeTab === 'following' && !providerConnected ? (
                            <div className="flex items-center justify-center h-full">
                                <div className="text-center glass-panel p-6 max-w-sm">
                                    <h3 className="text-base font-bold text-textPrimary mb-1">Not Connected</h3>
                                    <p className="text-textSecondary text-sm mb-4">
                                        Connect your {providerLabel(providerFilter as ProviderId)} account to see the channels you follow.
                                    </p>
                                    <div className="flex justify-center">
                                        <PlatformLoginButton
                                            provider={providerFilter as ProviderId}
                                            onClick={() => void connectPlatformAccount(providerFilter as 'kick' | 'youtube')}
                                            busy={providerConnecting}
                                        />
                                    </div>
                                </div>
                            </div>
                        ) : displayStreams.length === 0 && favoritesSectionCount === 0 && categorySearchResults.length === 0 && (activeTab !== 'search' || offlineSearchResults.length === 0) ? (
                            <div className="flex items-center justify-center h-full">
                                <div className="text-center glass-panel p-6 max-w-sm">
                                    <h3 className="text-base font-bold text-textPrimary mb-1">
                                        {providerError
                                            ? 'Could Not Load Streams'
                                            : isProviderView && activeTab === 'following' && providerFollows.every((f) => f.provider !== providerFilter)
                                                ? `No ${providerLabel(providerFilter as ProviderId)} Channels Yet`
                                                : activeTab === 'following' ? 'No Live Streams' : activeTab === 'recommended' ? 'No Streams' : 'No Results'}
                                    </h3>
                                    <p className="text-textSecondary text-sm">
                                        {isProviderView
                                            ? providerError
                                                ? `${providerLabel(providerFilter as ProviderId)} could not be reached right now.`
                                                : activeTab === 'following'
                                                    ? providerFollows.every((f) => f.provider !== providerFilter)
                                                        // Nothing here yet because the account isn't connected —
                                                        // say that, rather than implying we checked and found
                                                        // nobody live.
                                                        ? `Connect your ${providerLabel(providerFilter as ProviderId)} account in Settings to see the channels you follow.`
                                                        : `None of the ${providerLabel(providerFilter as ProviderId)} channels you follow are live.`
                                                    : `Nothing live on ${providerLabel(providerFilter as ProviderId)} right now.`
                                            : activeTab === 'following'
                                            ? 'None of your followed channels are live.'
                                            : activeTab === 'recommended'
                                                ? 'Could not load streams.'
                                                : searchMode === 'categories'
                                                    ? `No categories found for "${searchQuery}".`
                                                    : `No channels found for "${searchQuery}".`}
                                    </p>
                                    {/* Somewhere to go instead of a dead end: the platform's
                                        directory works whether or not an account is connected.
                                        Only shown once CONNECTED — otherwise it competed with
                                        the login panel above, offering to browse away from the
                                        thing the user was being asked to do. */}
                                    {isProviderView && activeTab === 'following' && !providerError && providerConnected && (
                                        <button
                                            onClick={() => setActiveTab('browse')}
                                            className="glass-button mt-4 px-4 py-2 text-sm font-medium rounded-lg transition-all hover:scale-105 mx-auto"
                                        >
                                            Browse {providerLabel(providerFilter as ProviderId)}
                                        </button>
                                    )}
                                    {/* Connect prompt for the platform being VIEWED. Browse and
                                        search do still load signed out — the catalog is real and
                                        hiding it would remove working product — but they must not
                                        be silent about the account, and must never offer Twitch's
                                        login on a Kick or YouTube surface. */}
                                    {isProviderView && !providerConnected && activeTab !== 'following' && (
                                        <div className="mt-4 pt-4 border-t border-borderSubtle">
                                            <p className="text-textSecondary text-xs mb-3">
                                                Connect {providerLabel(providerFilter as ProviderId)} to follow channels and chat
                                            </p>
                                            <div className="flex justify-center">
                                                <PlatformLoginButton
                                                    provider={providerFilter as ProviderId}
                                                    onClick={() => void connectPlatformAccount(providerFilter as 'kick' | 'youtube')}
                                                    busy={providerConnecting}
                                                />
                                            </div>
                                        </div>
                                    )}
                                    {/* Twitch's own prompt, gated to Twitch surfaces. It used to
                                        render on a Kick or YouTube empty state too. */}
                                    {!isAuthenticated && !isProviderView && activeTab === 'recommended' && (
                                        <div className="mt-4 pt-4 border-t border-borderSubtle">
                                            <p className="text-textSecondary text-xs mb-3">
                                                Log in for a better experience
                                            </p>
                                            <div className="flex justify-center">
                                                <PlatformLoginButton
                                                    provider="twitch"
                                                    onClick={loginToTwitch}
                                                    busy={isLoading}
                                                    busyLabel="Logging in…"
                                                />
                                            </div>
                                        </div>
                                    )}
                                </div>
                            </div>
                        ) : (
                            <>
                                {activeTab === 'search' && searchMode === 'categories' && categorySearchResults.length > 0 && (
                                    <div className="grid grid-cols-3 sm:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 2xl:grid-cols-8 gap-3">
                                        {categorySearchResults.map(game => renderCategoryCard(game))}
                                    </div>
                                )}
                                {/* FAVOURITES, pulled out above the rest.
                                    Not a filter over your follows: a favourite may
                                    be a channel you follow nowhere, which is exactly
                                    why it lives on this tab rather than being lost
                                    between Discover pages. The count sits on the
                                    section rather than the tab badge, which still
                                    means "live channels you follow". */}
                                {/* Pick up where you left off, above everything
                                    else on the tab. A rail, so an unfinished VOD
                                    never pushes live channels down the page, and
                                    self-hiding when there is nothing to resume. */}
                                {activeTab === 'following' && <ContinueWatchingRow />}
                                <LayoutGroup>
                                {favoritesTab && favoritesSectionCount > 0 && (
                                    <div className="mb-6 relative isolate rounded-2xl px-2 pt-2 -mx-2">
                                        {/* Film grain, and nothing else: the shelf is marked by
                                            texture rather than colour. A pink wash was tried here
                                            and cut - see the note on FAVORITES_GRAIN_MASK.
                                            `-left-2 -right-2` on top of the section's own `-mx-2`
                                            reaches the scroll container's padding EDGE, so the
                                            band runs the full width of the app instead of
                                            stopping 8px short of it. Exactly the padding, not
                                            more: overflowing right past it would put a horizontal
                                            scrollbar on the list. */}
                                        <div
                                            aria-hidden
                                            className="absolute -left-2 -right-2 -top-20 -bottom-24 -z-10 pointer-events-none"
                                            style={{
                                                backgroundImage: "url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='140' height='140'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.8' numOctaves='2' stitchTiles='stitch'/%3E%3CfeColorMatrix type='matrix' values='0 0 0 0 1 0 0 0 0 1 0 0 0 0 1 0 0 0 0.5 0'/%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)'/%3E%3C/svg%3E\")",
                                                opacity: 0.05,
                                                mixBlendMode: 'overlay',
                                                WebkitMaskImage: FAVORITES_GRAIN_MASK,
                                                maskImage: FAVORITES_GRAIN_MASK,
                                            }}
                                        />
                                        {/* Same identity as the sidebar shelf, same restraint:
                                            the filled pink heart carries the section, the label
                                            stays house-grey, and the count picks up a whisper of
                                            the pink so it reads as part of the mark. */}
                                        <div className="pb-3 px-2 flex items-center gap-2">
                                            <Heart
                                                size={14}
                                                fill="url(#glass-heart-fill)"
                                                stroke="url(#glass-heart-stroke)"
                                                strokeWidth={1.5}
                                                className="drop-shadow-[0_4px_8px_color-mix(in_srgb,var(--color-highlight-pink)_50%,transparent)]"
                                            />
                                            <h3 className="text-sm font-semibold text-textSecondary uppercase tracking-wide">
                                                Favorites
                                            </h3>
                                            <span
                                                className="text-xs font-semibold"
                                                style={{ color: 'color-mix(in srgb, var(--color-highlight-pink) 55%, var(--color-text-secondary))' }}
                                            >
                                                {liveFavorites.length}
                                            </span>
                                        </div>
                                        {/* LIVE only, by design. This shelf answers "who can I
                                            watch right now"; an offline favourite is not that, and
                                            putting one here would make the section's own count
                                            mean two different things. Offline favourites are still
                                            reachable in the Offline Channels roster below. */}
                                        <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 2xl:grid-cols-5 gap-3">
                                            <AnimatePresence mode="popLayout" initial={false}>
                                                {liveFavorites.map(renderStreamCard)}
                                            </AnimatePresence>
                                        </div>
                                        {/* Feathered at both ends, not a full-bleed rule. The wash
                                            above is a centred radial that fades out toward the
                                            edges, so a hairline running edge to edge at flat
                                            opacity terminated hard against nothing and read as a
                                            cut. Masked rather than recoloured so it keeps the
                                            house border token and stays 1px. */}
                                        {(displayStreams.length > 0 || offlineChannels.length > 0) && (
                                            <div
                                                className="mt-6 h-px bg-borderSubtle/70"
                                                style={{
                                                    WebkitMaskImage:
                                                        'linear-gradient(to right, transparent, black 26%, black 74%, transparent)',
                                                    maskImage:
                                                        'linear-gradient(to right, transparent, black 26%, black 74%, transparent)',
                                                }}
                                            />
                                        )}

                                    </div>
                                )}
                                {displayStreams.length > 0 && !(activeTab === 'search' && searchMode === 'categories') && (
                                    <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 2xl:grid-cols-5 gap-3">
                                    {/* Switching platforms replaces most of this grid, and
                                        without exits the old set vanished on the same frame the
                                        new one appeared. `popLayout` pulls a leaving card out of
                                        flow immediately, so the cards that survive the switch
                                        (every Twitch card when you go from All to Twitch) glide
                                        into their new positions on the layout spring each card
                                        already carries, instead of jumping.

                                        `initial={false}` so a cold start paints the first grid
                                        instantly: the entrance is for cards arriving into a grid
                                        that is already on screen, not for the first one. */}
                                    <AnimatePresence mode="popLayout" initial={false}>
                                        {displayStreams.map(renderStreamCard)}
                                    </AnimatePresence>
                                    </div>
                                )}
                                </LayoutGroup>

                                {/* Offline Followed Channels Section */}
                                {/* Twitch's offline follows. Hidden while scoped to another
                                    platform — they are Twitch channels, and listing them
                                    under a Kick Following tab is simply the wrong list. */}
                                {activeTab === 'following' && offlineChannels.length > 0 && (
                                    <div className={displayStreams.length > 0 ? "mt-6 pt-4 relative" : "pt-2"}>
                                        {displayStreams.length > 0 && (
                                            <div className="absolute top-0 left-0 right-0 h-px bg-borderSubtle/30" />
                                        )}
                                        <div className="col-span-full pb-3 px-2 flex justify-between items-center">
                                            <h3 className="text-sm font-semibold text-textSecondary uppercase tracking-wide flex items-center gap-2">
                                                <User size={14} className="text-textSecondary/70" />
                                                Offline Channels
                                            </h3>
                                        </div>
                                        <div className="flex flex-wrap gap-3 px-2 pb-6 relative z-0 mt-2">
                                            {[...offlineChannels].sort((a, b) => {
                                                const timeA = offlineLastBroadcasts[a.id] ? new Date(offlineLastBroadcasts[a.id]!).getTime() : 0;
                                                const timeB = offlineLastBroadcasts[b.id] ? new Date(offlineLastBroadcasts[b.id]!).getTime() : 0;
                                                return timeB - timeA;
                                            }).map(renderOfflineCard)}
                                            {isLoadingOfflineChannels && (
                                                <div className="flex items-center justify-center p-2 w-[180px] sm:w-[200px]">
                                                    <Loader2 size={16} className="animate-spin text-accent" />
                                                </div>
                                            )}
                                        </div>
                                    </div>
                                )}

                                {/* Offline Users Search Section */}
                                {activeTab === 'search' && offlineSearchResults.length > 0 && (
                                    <div className={displayStreams.length > 0 ? "mt-4 border-t border-borderSubtle/30 pt-4" : "pt-2"}>
                                        <div className="col-span-full pb-3 px-2">
                                            <h3 className="text-sm font-semibold text-textSecondary uppercase tracking-wide flex items-center gap-2">
                                                <User size={14} className="text-textSecondary/70" />
                                                Offline Channels
                                            </h3>
                                        </div>
                                        <div className="flex flex-wrap gap-3 px-2 pb-6 relative z-0">
                                            {offlineSearchResults.map((user) => (
                                                <button
                                                    key={user.id}
                                                    onClick={() => setProfileModalUser(user)}
                                                    className="flex items-center gap-3 px-3 py-2 rounded-xl glass-panel hover:bg-white/[0.05] border border-borderSubtle hover:border-accent/40 transition-all text-left shadow-sm group w-[180px] sm:w-[200px]"
                                                >
                                                    <div className="w-10 h-10 rounded-full bg-glass flex items-center justify-center overflow-hidden ring-1 ring-borderSubtle group-hover:ring-accent/40 flex-shrink-0">
                                                        {user.thumbnail_url ? (
                                                            <img src={user.thumbnail_url} alt={user.user_name} className="w-full h-full object-cover" />
                                                        ) : (
                                                            <User size={14} className="text-textSecondary" />
                                                        )}
                                                    </div>
                                                    <div className="flex-1 min-w-0">
                                                        <h4 className="text-sm font-semibold text-textPrimary truncate group-hover:text-accent transition-colors">
                                                            {user.user_name}
                                                        </h4>
                                                        <p className="text-[10px] text-textSecondary truncate">
                                                            {user.game_name || 'Channel'}
                                                        </p>
                                                    </div>
                                                </button>
                                            ))}
                                        </div>
                                    </div>
                                )}

                                {activeTab === 'recommended' && isLoadingMore && (
                                    <div className="flex justify-center items-center py-6">
                                        <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-accent"></div>
                                    </div>
                                )}
                                {activeTab === 'recommended' && !hasMoreRecommended && displayStreams.length > 0 && (
                                    <div className="text-center py-6">
                                        <p className="text-textSecondary text-xs">No more streams</p>
                                    </div>
                                )}
                            </>
                        )}
                    </>
                )}

                {/* Flying Gift Animation - flies up toward title bar */}
                {flyingDroplet && (
                    <div
                        className="fixed pointer-events-none z-50"
                        style={{
                            left: flyingDroplet?.x ?? 0,
                            top: flyingDroplet?.y ?? 0,
                            transform: 'translate(-50%, -50%)',
                        }}
                    >
                        <div className="animate-fly-up-fade">
                            <Gift
                                size={24}
                                className="gift-shimmer-gold"
                            />
                        </div>
                    </div>
                )}
            </div>
        </div>
    );
};

export default Home;

