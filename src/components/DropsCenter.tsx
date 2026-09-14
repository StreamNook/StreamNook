import { useEffect, useState, useMemo, useRef, useCallback } from 'react';
import { motion, AnimatePresence, LayoutGroup } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../stores/AppStore';
import { listen, emit } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Search, Gift, MonitorPlay, BarChart3, Package, ArrowDownUp, SlidersHorizontal, Check, ChevronDown, ChevronUp } from 'lucide-react';
import { usePluginUiRegistry, selectSlot } from '../plugins-ui/registry';
import { Dropdown } from './ui/Dropdown';
import { SegmentedSelect } from './settings/_primitives';
import {
    UnifiedGame, DropCampaign, DropProgress, DropsStatistics,
    DropProgressStatus, DropsDeviceCodeInfo, InventoryResponse, InventoryItem, CompletedDrop, TwitchStream
} from '../types';

import LoadingWidget from './LoadingWidget';
import GameCard from './drops/GameCard';
import GameDetailPanel from './drops/GameDetailPanel';
import DropsStatsTab from './drops/DropsStatsTab';
import DropsInventoryTab from './drops/DropsInventoryTab';
import { getAllUserBadgesWithEarned } from '../services/badgeService';
import { Tooltip } from './ui/Tooltip';

import { Logger } from '../utils/logger';
// Twitch SVG Icon Component
const TwitchIcon = ({ size = 20 }: { size?: number }) => (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor" xmlns="http://www.w3.org/2000/svg">
        <path d="M11.571 4.714h1.715v5.143H11.57zm4.715 0H18v5.143h-1.714zM6 0L1.714 4.286v15.428h5.143V24l4.286-4.286h3.428L22.286 12V0zm14.571 11.143l-3.428 3.428h-3.429l-3 3v-3H6.857V1.714h13.714Z" />
    </svg>
);

type Tab = 'games' | 'inventory' | 'stats' | 'settings';

interface DropsSettings {
    auto_claim_drops: boolean;
    auto_claim_channel_points: boolean;
    notify_on_drop_available: boolean;
    notify_on_drop_claimed: boolean;
    notify_on_points_claimed: boolean;
    check_interval_seconds: number;
    automation_enabled: boolean;
    priority_games: string[];
    excluded_games: string[];
    priority_mode: 'PriorityOnly' | 'EndingSoonest' | 'LowAvailFirst';
    watch_interval_seconds: number;
    favorite_games: string[];  // UI-only, for sorting/tracking - doesn't affect automation
    // Watch token allocation settings
    reserve_token_for_current_stream?: boolean;
    auto_reserve_on_watch?: boolean;
    priority_channels?: Array<{ channel_id: string; channel_login: string; display_name: string }>;
}

// A campaign has something mineable if any of its drops is watch-time earnable.
// Event/paid/sub/gift-only campaigns (no watch-time drop) are non-mineable; they
// only show when the grid's "All drops" view is on.
function campaignHasMineableDrop(c: DropCampaign): boolean {
    return (c.time_based_drops || []).some(d =>
        typeof d.is_collectible === 'boolean'
            ? d.is_collectible
            : (d.required_minutes_watched || 0) > 0 ||
              (d.progress?.required_minutes_watched || 0) > 0
    );
}

export default function DropsCenter() {
    // Data State
    const [unifiedGames, setUnifiedGames] = useState<UnifiedGame[]>([]);
    const [inventoryItems, setInventoryItems] = useState<InventoryItem[]>([]);
    const [completedDrops, setCompletedDrops] = useState<CompletedDrop[]>([]);
    const [statistics, setStatistics] = useState<DropsStatistics | null>(null);
    const [progress, setProgress] = useState<DropProgress[]>([]);
    const [, setEarnedBadgeIds] = useState<Set<string>>(new Set());
    const [earnedBadgeTitles, setEarnedBadgeTitles] = useState<Set<string>>(new Set());
    // Earned badge titles plus Twitch's global badge catalog. Used to tell a global
    // badge reward (e.g. "YOU GOT THIS") apart from an in-game item, since both can carry
    // a null distribution_type and the same generic quests asset URL.
    const [knownBadgeTitles, setKnownBadgeTitles] = useState<Set<string>>(new Set());
    const [isLoading, setIsLoading] = useState(true);
    const [, setError] = useState<string | null>(null);

    // Auth State
    const [isAuthenticated, setIsAuthenticated] = useState(false);
    const [isAuthenticating, setIsAuthenticating] = useState(false);
    const [deviceCodeInfo, setDeviceCodeInfo] = useState<DropsDeviceCodeInfo | null>(null);

    // Automation State
    const [dropProgress, setDropProgress] = useState<DropProgressStatus | null>(null);
    // Id of a plugin that provides drops automation, or null. When set, the
    // cockpit's collect controls drive the plugin through the general hooks
    // instead of the built-in automation. Core never names the plugin.
    const [externalDropsProviderId, setExternalDropsProviderId] = useState<string | null>(null);

    // UI State
    const [activeTab, setActiveTab] = useState<Tab>('games');
    // A drops-automation plugin contributes its automation settings panel into this
    // slot; the Settings tab exists only while one does. If the plugin is
    // disabled while the tab is open, fall back to Games so the view never
    // strands on an empty tab.
    const settingsSlots = usePluginUiRegistry(selectSlot('drops.settings'));
    useEffect(() => {
        if (activeTab === 'settings' && settingsSlots.length === 0) setActiveTab('games');
    }, [activeTab, settingsSlots.length]);
    const [searchTerm, setSearchTerm] = useState('');
    // Game grid sort: 'recommended' = relevance order, 'newest'/'oldest' = by most-recent campaign release.
    const [sortMode, setSortMode] = useState<'recommended' | 'newest' | 'oldest'>('recommended');
    // When on, include campaigns with no watch-time (mineable) drop — paid, sub,
    // gift, and event-only drops — instead of hiding them from the grid.
    const [showAllDrops, setShowAllDrops] = useState(false);
    // Fully-collected games live in their own collapsible section under the
    // grid (mirrors the Inventory tab's Completed Drops section) instead of
    // sinking to the bottom of the active grid.
    const [showCompletedGames, setShowCompletedGames] = useState(false);
    const [selectedGame, setSelectedGame] = useState<UnifiedGame | null>(null);
    const [, setIsLoadingGameDetail] = useState(false);
    const { addToast, setShowDropsOverlay, currentUser, dropsSearchTerm, setDropsSearchTerm } = useAppStore();
    


    // Channel Picker State

    // Settings State
    const [dropsSettings, setDropsSettings] = useState<DropsSettings | null>(null);

    // Ref for the games container to scroll to top when automation starts
    const gamesContainerRef = useRef<HTMLDivElement>(null);

    // Track the previously automation game to detect when automation starts
    const prevAutomationGameRef = useRef<string | null>(null);

    // Track previous campaign IDs for notification detection
    const prevCampaignIdsRef = useRef<Set<string>>(new Set());

    // Derived state for filtering AND sorting (favorites first)
    const filteredGames = useMemo(() => {
        const favoriteGames = dropsSettings?.favorite_games || [];
        // The actively-automation game pins above everything, even favorites: it's
        // the one thing happening right now. Derived straight from live
        // dropProgress so a reopened overlay restores the pin immediately,
        // not only after the next status push.
        const progressGameName = (dropProgress?.active
            ? dropProgress.current_drop?.game_name || dropProgress.current_channel?.game_name
            : null)?.toLowerCase() || null;
        let games = unifiedGames;

        // "Mineable only" (default): drop fully non-mineable campaigns from each
        // game, then any game with nothing left to mine. "All drops" keeps them,
        // so paid/sub/gift/event-only drops are shown (with their unlock text in
        // the detail panel). Filtered at render so toggling never reloads.
        if (!showAllDrops) {
            games = games
                .map(g => {
                    const mineable = g.active_campaigns.filter(campaignHasMineableDrop);
                    return mineable.length === g.active_campaigns.length
                        ? g
                        : { ...g, active_campaigns: mineable, total_active_drops: mineable.reduce((n, c) => n + (c.time_based_drops?.length || 0), 0) };
                })
                .filter(g => g.active_campaigns.length > 0);
        }

        // Apply search filter
        if (searchTerm) {
            const lowerSearch = searchTerm.toLowerCase();
            games = games.filter(game =>
                game.name.toLowerCase().includes(lowerSearch) ||
                game.active_campaigns.some(c => c.name.toLowerCase().includes(lowerSearch))
            );
        }
        
        // A game's "release recency" = the most recent campaign start among its
        // active campaigns (newest campaign to come out for that game).
        const releaseTime = (g: UnifiedGame) => {
            let t = 0;
            for (const c of g.active_campaigns) {
                const ms = Date.parse(c.start_at);
                if (!Number.isNaN(ms) && ms > t) t = ms;
            }
            return t;
        };

        // Sort: actively-automation game first, then still-collectible favorites, then by the selected sort mode.
        return [...games].sort((a, b) => {
            // Actively-automation game pinned at the very top, above favorites.
            const aAutomation = progressGameName !== null && a.name.toLowerCase() === progressGameName;
            const bAutomation = progressGameName !== null && b.name.toLowerCase() === progressGameName;
            if (aAutomation !== bAutomation) return aAutomation ? -1 : 1;

            // A favorite that's fully claimed is "done", so it drops out of the top
            // pin and sinks to the bottom with the other completed games.
            const aIsFavorite = favoriteGames.some(pg => pg.toLowerCase() === a.name.toLowerCase()) && !a.all_drops_claimed;
            const bIsFavorite = favoriteGames.some(pg => pg.toLowerCase() === b.name.toLowerCase()) && !b.all_drops_claimed;

            // Active favorites first
            if (aIsFavorite !== bIsFavorite) return aIsFavorite ? -1 : 1;

            // Explicit date sort: newest or oldest by most-recent campaign release.
            if (sortMode === 'newest' || sortMode === 'oldest') {
                const diff = releaseTime(b) - releaseTime(a); // newest-first baseline
                if (diff !== 0) return sortMode === 'newest' ? diff : -diff;
                return a.name.localeCompare(b.name);
            }

            // Recommended (default) relevance order:
            // Automation games next
            if (a.active !== b.active) return a.active ? -1 : 1;
            // Completed games (all drops claimed) go to bottom
            if (a.all_drops_claimed !== b.all_drops_claimed) return a.all_drops_claimed ? 1 : -1;
            // Games with claimable drops next
            if (a.has_claimable !== b.has_claimable) return a.has_claimable ? -1 : 1;
            // Then by number of active campaigns
            if (a.active_campaigns.length !== b.active_campaigns.length) {
                return b.active_campaigns.length - a.active_campaigns.length;
            }
            return a.name.localeCompare(b.name);
        });
    }, [unifiedGames, searchTerm, dropsSettings?.favorite_games, sortMode, dropProgress, showAllDrops]);

    // Fetch earned badges on mount for badge drop ownership verification
    useEffect(() => {
        const fetchEarnedBadges = async () => {
            if (!currentUser?.user_id || !currentUser?.login) return;
            
            try {
                Logger.debug('[DropsCenter] Fetching earned badges for user:', currentUser.login);
                const badges = await getAllUserBadgesWithEarned(
                    currentUser.user_id,
                    currentUser.login,
                    currentUser.user_id, // Use user's own channel ID
                    currentUser.login // Use user's own channel name
                );
                
                // Extract earned badge IDs from the flat badge structure
                const earnedIds = new Set<string>();
                const badgeTitles = new Set<string>();
                badges.earnedBadges?.forEach((badge) => {
                    if (badge.id) earnedIds.add(badge.id);
                    if (badge.title) badgeTitles.add(badge.title.toLowerCase());
                });
                // Also include third-party badges
                badges.thirdPartyBadges?.forEach((badge) => {
                    if (badge.id) earnedIds.add(badge.id);
                    if (badge.title) badgeTitles.add(badge.title.toLowerCase());
                });
                
                setEarnedBadgeIds(earnedIds);
                setEarnedBadgeTitles(badgeTitles);
                Logger.debug('[DropsCenter] Loaded earned badge IDs:', earnedIds.size);
                Logger.debug('[DropsCenter] Loaded earned badge titles:', badgeTitles.size);
                Logger.debug('[DropsCenter] Sample badge titles:', Array.from(badgeTitles).slice(0, 5));

                // Add the global badge catalog so the rewards list can label an unearned
                // global badge correctly (in-game items aren't in the catalog).
                try {
                    const { invoke } = await import('@tauri-apps/api/core');
                    let globalBadges = await invoke<{ data?: Array<{ versions?: Array<{ title?: string }> }> } | null>('get_cached_global_badges');
                    if (!globalBadges) {
                        await invoke('prefetch_global_badges').catch(() => {});
                        globalBadges = await invoke<{ data?: Array<{ versions?: Array<{ title?: string }> }> } | null>('get_cached_global_badges');
                    }
                    const known = new Set<string>(badgeTitles);
                    globalBadges?.data?.forEach(set => set.versions?.forEach(v => {
                        const t = v.title?.toLowerCase().trim();
                        if (t) known.add(t);
                    }));
                    setKnownBadgeTitles(known);
                    Logger.debug('[DropsCenter] Known badge titles (earned + global):', known.size);
                } catch (e) {
                    setKnownBadgeTitles(new Set(badgeTitles));
                    Logger.warn('[DropsCenter] Failed to load global badge catalog for reward labeling:', e);
                }
            } catch (err) {
                Logger.error('[DropsCenter] Failed to fetch earned badges:', err);
            }
        };
        
        fetchEarnedBadges();
    }, [currentUser?.user_id, currentUser?.login]);

    // ---- Authentication Logic ----
    const checkAuthentication = async () => {
        try {
            const authenticated = await invoke<boolean>('is_drops_authenticated');
            setIsAuthenticated(authenticated);
            return authenticated;
        } catch (err) {
            Logger.error('Failed to check drops authentication:', err);
            setIsAuthenticated(false);
            return false;
        }
    };

    const startDropsLogin = async () => {
        try {
            setIsAuthenticating(true);
            setError(null);
            const deviceInfo = await invoke<DropsDeviceCodeInfo>('start_drops_device_flow');
            setDeviceCodeInfo(deviceInfo);

            // Bound to the active account's web profile (Rust), so it reuses the
            // main login's twitch.tv session — authorize only, no re-login.
            await invoke('open_drops_login_window', { url: deviceInfo.verification_uri });

            pollForToken(deviceInfo);
        } catch (err) {
            Logger.error('Failed to start drops login:', err);
            setError(err instanceof Error ? err.message : String(err));
            setIsAuthenticating(false);
        }
    };

    const pollForToken = async (deviceInfo: DropsDeviceCodeInfo) => {
        try {
            await invoke('poll_drops_token', {
                deviceCode: deviceInfo.device_code,
                interval: deviceInfo.interval,
                expiresIn: deviceInfo.expires_in,
            });

            // Dismiss the in-app drops login overlay
            try {
                await invoke('close_login_overlay', { label: 'drops-login' });
            } catch (closeErr) {
                Logger.warn('[DropsCenter] Failed to close drops login overlay:', closeErr);
            }

            setIsAuthenticated(true);
            setIsAuthenticating(false);
            setDeviceCodeInfo(null);
            addToast('Drops login successful!', 'success');
            await loadDropsData();
        } catch (err) {
            Logger.error('Failed to complete drops login:', err);
            setError(err instanceof Error ? err.message : String(err));
            setIsAuthenticating(false);
            
            // Also dismiss the drops login overlay on error
            try {
                await invoke('close_login_overlay', { label: 'drops-login' });
            } catch { /* overlay may not exist */ }
        }
    };

    const handleDropsLogout = async () => {
        try {
            await invoke('drops_logout');
            setIsAuthenticated(false);
            setUnifiedGames([]);
            setProgress([]);
            setStatistics(null);
            setSelectedGame(null);
        } catch (err) {
            Logger.error('Failed to logout from drops:', err);
        }
    };

    // ---- Action Handlers ----
    const handleClaimDrop = async (dropId: string, dropInstanceId?: string) => {
        try {
            Logger.debug('[DropsCenter] Claiming drop:', dropId, 'with dropInstanceId:', dropInstanceId);
            await invoke('claim_drop', { dropId, dropInstanceId });
            addToast('Drop claimed successfully!', 'success');

            // Mark the drop claimed locally for instant feedback.
            const nextProgress = progress.map(p =>
                p.drop_id === dropId ? { ...p, is_claimed: true } : p
            );
            setProgress(nextProgress);

            // A claim only moves a reward into your inventory. It does NOT change the
            // active campaigns or the live automation-progress cache, so we deliberately
            // skip the full loadDropsData() reload here. That reload would blank the
            // panel behind a spinner AND re-fetch campaigns, which clears the backend's
            // live progress map and makes the title-bar automation progress snap backwards
            // until it slowly re-accumulates. Instead: refresh only the inventory
            // (silently) and patch the claimed game's flags in place.
            const inventoryData = await invoke<InventoryResponse>('get_drops_inventory').catch(() => null);
            if (inventoryData?.items) setInventoryItems(inventoryData.items);
            if (inventoryData?.completed_drops) setCompletedDrops(inventoryData.completed_drops);

            // Owned-reward sets from the freshly fetched inventory, matching how the
            // full rebuild decides ownership: a reward counts as earned by its claim
            // flag, by drop id, by benefit id, or (badges only) by benefit name.
            const ownedBenefitIds = new Set<string>((inventoryData?.completed_drops || []).map(d => d.id));
            const ownedBenefitNames = new Set<string>(
                (inventoryData?.completed_drops || []).map(d => (d.name || '').toLowerCase().trim()).filter(Boolean)
            );
            const ownedDropIds = new Set<string>();
            inventoryData?.items?.forEach(item => {
                item.campaign.time_based_drops.forEach(drop => {
                    if (drop.progress?.is_claimed === true) ownedDropIds.add(drop.id);
                });
            });
            // Name matching is badge-only: a held badge can't be earned again, but
            // a consumable reward reissued under a new campaign instance (same
            // name, new benefit ids) is genuinely earnable again and must not be
            // counted as already claimed.
            const ownedByName = (name?: string) => {
                const n = (name || '').toLowerCase().trim();
                return !!n && ownedBenefitNames.has(n) && knownBadgeTitles.has(n);
            };

            const gameOwnsDrop = (game: UnifiedGame) =>
                game.active_campaigns.some(c => c.time_based_drops.some(d => d.id === dropId));

            const recomputeGame = (game: UnifiedGame): UnifiedGame => {
                let hasClaimable = false;
                let totalDrops = 0;
                let claimedCount = 0;
                game.active_campaigns.forEach(campaign => {
                    campaign.time_based_drops.forEach(drop => {
                        totalDrops++;
                        const dp = nextProgress.find(p => p.drop_id === drop.id) || drop.progress;
                        const hasCurrentProgress = !!dp && ((dp.current_minutes_watched || 0) > 0 || dp.is_claimed === true);
                        // An inventory drop flagged is_claimed is a proven claim of THIS
                        // drop instance (reissues mint new drop ids), so it counts even
                        // while a stale progress record still carries watch minutes.
                        // Benefit id/name matching stays gated on no-current-progress.
                        const owned = dp?.is_claimed === true
                            || ownedDropIds.has(drop.id)
                            || (!hasCurrentProgress && (
                                drop.benefit_edges?.some(b =>
                                    ownedBenefitIds.has(b.id) || ownedByName(b.name)
                                ) ?? false
                            ));
                        if (owned) {
                            claimedCount++;
                        } else if (dp && dp.required_minutes_watched > 0 && dp.current_minutes_watched >= dp.required_minutes_watched) {
                            hasClaimable = true;
                        }
                    });
                });
                const freshClaimed = inventoryData?.items
                    ? inventoryData.items
                        .filter(it => it.campaign.game_id === game.id)
                        .reduce((sum, it) => sum + it.claimed_drops, 0)
                    : game.total_claimed;
                return {
                    ...game,
                    has_claimable: hasClaimable,
                    all_drops_claimed: totalDrops > 0 && claimedCount === totalDrops,
                    total_claimed: freshClaimed,
                };
            };

            // Re-sort with the same ordering loadDropsData uses, so a now fully-claimed
            // game sinks to the bottom without a reload.
            const sortGames = (a: UnifiedGame, b: UnifiedGame) => {
                if (a.active !== b.active) return a.active ? -1 : 1;
                if (a.all_drops_claimed !== b.all_drops_claimed) return a.all_drops_claimed ? 1 : -1;
                if (a.has_claimable !== b.has_claimable) return a.has_claimable ? -1 : 1;
                if (a.active_campaigns.length !== b.active_campaigns.length) {
                    return b.active_campaigns.length - a.active_campaigns.length;
                }
                return a.name.localeCompare(b.name);
            };

            setUnifiedGames(prev =>
                prev.map(g => (gameOwnsDrop(g) ? recomputeGame(g) : g)).sort(sortGames)
            );
            setSelectedGame(prev => (prev && gameOwnsDrop(prev) ? recomputeGame(prev) : prev));
        } catch (err) {
            Logger.error('Failed to claim drop:', err);
            addToast('Failed to claim drop', 'error');
        }
    };

    // ---- External drops provider routing ----
    // Track whether an external provider (an opt-in plugin) is present, so the
    // provider-only controls route to it. The global DropProgressController
    // drives the 'drop-progress' event the cockpit consumes below (from native
    // watch-to-earn or a provider), so there is nothing to translate here.
    useEffect(() => {
        let disposed = false;
        const refreshProvider = async () => {
            try {
                const id = await invoke<string | null>('plugins_provides', { feature: 'drops.automation' });
                if (!disposed) setExternalDropsProviderId(id ?? null);
            } catch { /* plugin host unavailable */ }
        };
        refreshProvider();

        const unlisteners: (() => void)[] = [];
        const setup = async () => {
            const unState = await listen('plugin://state-changed', () => refreshProvider());
            if (disposed) {
                unState();
            } else {
                unlisteners.push(unState);
            }
        };
        setup();
        return () => {
            disposed = true;
            unlisteners.forEach((u) => u());
        };
    }, []);

    // Starting is native (the card's "Watch" link to the category) or the
    // provider's own per-card control, so core issues no start action. Stop is
    // the one provider-driven control core still issues, and only when a provider
    // is present (the Stop button is hidden otherwise).
    const stopAutomation = async () => {
        if (!externalDropsProviderId) return;
        await invoke('plugins_invoke_action', { action: 'drops.stop', args: {} });
    };

    const handleStopAutomation = async () => {
        try {
            // Immediately update local state to reflect stopped automation
            setDropProgress(prev => prev ? {
                ...prev,
                active: false,
                current_drop: null,
                current_channel: null,
                current_campaign: null
            } : null);

            // Clear ALL in-progress entries to prevent stale data when switching games
            // We completely reset and let the new automation session repopulate fresh data
            setProgress([]);

            // Then call the backend to actually stop
            await stopAutomation();
            addToast('Automation stopped', 'info');
        } catch (err) {
            Logger.error('Failed to stop automation:', err);
        }
    };

    const updateDropsSettings = async (newSettings: Partial<DropsSettings>) => {
        try {
            const current = dropsSettings || {
                auto_claim_drops: true,
                auto_claim_channel_points: true,
                notify_on_drop_available: true,
                notify_on_drop_claimed: true,
                notify_on_points_claimed: false,
                check_interval_seconds: 60,
                automation_enabled: false,
                priority_games: [],
                excluded_games: [],
                priority_mode: 'PriorityOnly' as const,
                watch_interval_seconds: 20,
                favorite_games: [],
                priority_channels: [],
                prefer_favorites: false,
            };
            const updatedSettings = { ...current, ...newSettings };

            await invoke('update_drops_settings', { settings: updatedSettings });
            setDropsSettings(updatedSettings);

            useAppStore.getState().updateSettings({
                ...useAppStore.getState().settings,
                drops: updatedSettings
            });
        } catch (err) {
            Logger.error('Failed to update drops settings:', err);
            addToast('Failed to save settings', 'error');
        }
    };

    const handleStreamClick = (channelName: string, streamInfo?: TwitchStream) => {
        setShowDropsOverlay(false);
        setSelectedGame(null);
        // Pass the live stream object (carries game_name) just like a stream card
        // click does, so the drop-progress badge lights up; without it the controller
        // can start with an empty category and never match the campaign.
        void useAppStore.getState().startStream(channelName, streamInfo);
    };

    // Toggle favorite (add/remove from favorite_games - visual only, doesn't affect automation)
    const handleToggleFavorite = async (gameName: string) => {
        if (!dropsSettings) return;
        
        const currentFavorites = dropsSettings.favorite_games || [];
        const isCurrentlyFavorite = currentFavorites.some(
            pg => pg.toLowerCase() === gameName.toLowerCase()
        );
        
        let newFavoriteGames: string[];
        if (isCurrentlyFavorite) {
            // Remove from favorites
            newFavoriteGames = currentFavorites.filter(
                pg => pg.toLowerCase() !== gameName.toLowerCase()
            );
            addToast(`Removed ${gameName} from favorites`, 'info');
        } else {
            // Add to favorites
            newFavoriteGames = [...currentFavorites, gameName];
            addToast(`Added ${gameName} to favorites`, 'success');
        }
        
        await updateDropsSettings({ favorite_games: newFavoriteGames });
    };

    // Handle game selection - polls inventory for fresh progress data
    const handleGameSelect = async (game: UnifiedGame | null) => {
        // If deselecting (clicking same game), just close
        if (game === null || (selectedGame && selectedGame.id === game.id)) {
            setSelectedGame(null);
            return;
        }

        // Set loading state and selected game
        setIsLoadingGameDetail(true);
        setSelectedGame(game);

        try {
            Logger.debug('[DropsCenter] Fetching fresh inventory for game:', game.name);
            
            // Poll inventory to get the latest progress data
            const inventoryData = await invoke<InventoryResponse>('get_drops_inventory');
            
            if (inventoryData?.items) {
                Logger.debug('[DropsCenter] Got fresh inventory with', inventoryData.items.length, 'items');
                
                // Update global inventory items state
                setInventoryItems(inventoryData.items);
                
                // Extract progress data from inventory and merge into progress state
                const progressFromInventory: DropProgress[] = [];
                
                inventoryData.items.forEach(item => {
                    item.campaign.time_based_drops.forEach(drop => {
                        if (drop.progress) {
                            progressFromInventory.push({
                                campaign_id: item.campaign.id,
                                drop_id: drop.id,
                                current_minutes_watched: drop.progress.current_minutes_watched,
                                required_minutes_watched: drop.progress.required_minutes_watched,
                                is_claimed: drop.progress.is_claimed,
                                last_updated: drop.progress.last_updated,
                                drop_instance_id: drop.progress.drop_instance_id,
                            });
                        }
                    });
                });
                
                Logger.debug('[DropsCenter] Extracted', progressFromInventory.length, 'progress entries from inventory');
                
                // Merge inventory progress with existing progress (inventory takes priority for matching drops)
                setProgress(prevProgress => {
                    const mergedProgress = [...prevProgress];
                    
                    progressFromInventory.forEach(inventoryProg => {
                        const existingIndex = mergedProgress.findIndex(p => p.drop_id === inventoryProg.drop_id);
                        
                        if (existingIndex >= 0) {
                            // Update existing entry with inventory data (inventory is authoritative)
                            mergedProgress[existingIndex] = {
                                ...mergedProgress[existingIndex],
                                ...inventoryProg,
                            };
                        } else {
                            // Add new entry from inventory
                            mergedProgress.push(inventoryProg);
                        }
                    });
                    
                    Logger.debug('[DropsCenter] Merged progress now has', mergedProgress.length, 'entries');
                    return mergedProgress;
                });
                
                // Update the selected game with fresh inventory items
                const freshInventoryForGame = inventoryData.items.filter(item => {
                    // Match by game name (case-insensitive) or game_id
                    const itemGameName = item.campaign.game_name?.toLowerCase() || '';
                    const gameNameLower = game.name.toLowerCase();
                    return itemGameName === gameNameLower || item.campaign.game_id === game.id;
                });
                
                if (freshInventoryForGame.length > 0) {
                    Logger.debug('[DropsCenter] Found', freshInventoryForGame.length, 'inventory items for game:', game.name);
                    
                    // Update the selected game with fresh inventory
                    setSelectedGame(prevGame => {
                        if (!prevGame) return null;
                        return {
                            ...prevGame,
                            inventory_items: freshInventoryForGame,
                            total_claimed: freshInventoryForGame.reduce((sum, item) => sum + item.claimed_drops, 0),
                        };
                    });
                    
                    // Also update this game in the unifiedGames array
                    setUnifiedGames(prevGames => {
                        return prevGames.map(g => {
                            if (g.id === game.id) {
                                return {
                                    ...g,
                                    inventory_items: freshInventoryForGame,
                                    total_claimed: freshInventoryForGame.reduce((sum, item) => sum + item.claimed_drops, 0),
                                };
                            }
                            return g;
                        });
                    });
                }
            }
        } catch (err) {
            Logger.error('[DropsCenter] Failed to fetch inventory for game:', err);
            // Don't show error toast - we still show the panel with cached data
        } finally {
            setIsLoadingGameDetail(false);
        }
    };

    // Auto-apply dropsSearchTerm when navigated to from a specific deep-link
    useEffect(() => {
        if (dropsSearchTerm && unifiedGames.length > 0) {
            setSearchTerm(dropsSearchTerm);
            
            const lowerSearch = dropsSearchTerm.toLowerCase();
            const exactMatch = unifiedGames.find(g => g.name.toLowerCase() === lowerSearch);
            
            if (exactMatch) {
                setTimeout(() => handleGameSelect(exactMatch), 50);
            } else {
                const partialMatches = unifiedGames.filter(g => g.name.toLowerCase().includes(lowerSearch));
                if (partialMatches.length === 1) {
                    setTimeout(() => handleGameSelect(partialMatches[0]), 50);
                }
            }
            
            // Clear the search term from store so it doesn't re-trigger when returning
            setDropsSearchTerm('');
        }
    // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [dropsSearchTerm, unifiedGames]);

    // The "Connect account" button opens the publisher's link page in the external browser, so
    // there's no in-app close event to hang a refresh on. Instead: when the user clicks Connect we
    // arm a pending flag, and re-check connection status the next time the app regains focus (they've
    // come back from the browser). Gated by the flag so we don't refetch on every alt-tab.
    const pendingConnectRef = useRef(false);

    // Re-check status without a full reload. loadDropsData() here would blank the panel behind a
    // spinner AND clear the backend's live progress map (see handleClaimDrop); instead we fetch fresh
    // campaigns via a command that skips the progress sync, then patch the is_account_connected /
    // account_link flags in place — on both the grid and the open panel (selectedGame is a snapshot,
    // not a live reference into unifiedGames).
    const refreshConnectionStatus = useCallback(async () => {
        const fresh = await invoke<DropCampaign[]>('refresh_drops_connection_status').catch(() => null);
        if (!fresh) return;
        const byId = new Map(fresh.map(c => [c.id, c]));
        const patchGame = (g: UnifiedGame): UnifiedGame => ({
            ...g,
            active_campaigns: g.active_campaigns.map(c => {
                const f = byId.get(c.id);
                return f
                    ? { ...c, is_account_connected: f.is_account_connected, account_link: f.account_link }
                    : c;
            }),
        });
        setUnifiedGames(prev => prev.map(patchGame));
        setSelectedGame(prev => (prev ? patchGame(prev) : prev));
    }, []);

    useEffect(() => {
        const arm = () => { pendingConnectRef.current = true; };
        window.addEventListener('drops-connect-initiated', arm);
        // The subscription resolves asynchronously; if this effect is cleaned
        // up first (StrictMode's double invoke, or a dependency change) the
        // late-arriving handle must be released, not stored, or the listener
        // outlives the effect.
        let cancelled = false;
        let unlisten: (() => void) | undefined;
        getCurrentWindow()
            .onFocusChanged(({ payload: focused }) => {
                if (focused && pendingConnectRef.current) {
                    pendingConnectRef.current = false;
                    refreshConnectionStatus();
                }
            })
            .then(u => {
                if (cancelled) u();
                else unlisten = u;
            })
            .catch(() => {});
        return () => {
            cancelled = true;
            window.removeEventListener('drops-connect-initiated', arm);
            unlisten?.();
        };
    }, [refreshConnectionStatus]);


    // ---- Data Loading & Merging Logic ----
    const loadDropsData = async () => {
        try {
            setIsLoading(true);
            setError(null);

            // IMPORTANT:
            // `get_active_drop_campaigns` is responsible for refreshing the backend's internal
            // drop progress map (via `DropsService::update_campaigns_and_progress`).
            // If we fetch `get_drop_progress` in parallel, it can race and return an empty
            // list, which makes the UI show 0 minutes for everything.
            //
            // So: fetch campaigns first, then fetch progress.
            const [campaignsData, statsData, inventoryData] = await Promise.all([
                invoke<DropCampaign[]>('get_active_drop_campaigns').catch(() => [] as DropCampaign[]),
                invoke<DropsStatistics>('get_drops_statistics').catch(() => null),
                invoke<InventoryResponse>('get_drops_inventory').catch(() => null),
            ]);

            const progressData = await invoke<DropProgress[]>('get_drop_progress').catch(() => [] as DropProgress[]);

            // Seed automation status from the bridge-cached live status: a plugin
            // powering automation reports through the bridge into the store and keeps
            // it there even while this overlay is closed, so a reopened overlay
            // immediately shows what is being collected.
            const liveStatus = useAppStore.getState().liveDropProgress;
            if (liveStatus) {
                setDropProgress(liveStatus);
            }

            if (progressData) setProgress(progressData);
            if (statsData) setStatistics(statsData);
            if (inventoryData?.items) setInventoryItems(inventoryData.items);
            if (inventoryData?.completed_drops) {
                Logger.debug(`[DropsCenter] Found ${inventoryData.completed_drops.length} completed drops`);
                setCompletedDrops(inventoryData.completed_drops);
            }

            // Merge data into unified games
            const gamesMap = new Map<string, UnifiedGame>();

            const getOrCreateGame = (id: string, name: string, boxArt: string): UnifiedGame => {
                let game = gamesMap.get(id);
                if (!game) {
                    game = {
                        id,
                        name,
                        box_art_url: boxArt,
                        active_campaigns: [],
                        total_active_drops: 0,
                        drops_in_progress: 0,
                        inventory_items: [],
                        total_claimed: 0,
                        active: false,
                        has_claimable: false,
                        all_drops_claimed: false
                    };
                    gamesMap.set(id, game);
                }
                return game;
            };

            // Process Active Campaigns and merge progress data from inventory
            if (campaignsData) {
                campaignsData.forEach(campaign => {
                    // IMPORTANT: Merge progress data into each drop BEFORE the collectible
                    // check + adding to game, so embedded/inventory progress is counted.
                    const campaignWithProgress = {
                        ...campaign,
                        time_based_drops: campaign.time_based_drops.map(drop => {
                            // Find progress from progressData (real-time updates)
                            const prog = progressData?.find(p => p.drop_id === drop.id);
                            
                            // If no progress in progressData, check inventory items
                            let inventoryProgress = null;
                            if (!prog && inventoryData?.items) {
                                // Find matching inventory item for this campaign
                                const inventoryItem = inventoryData.items.find(item => 
                                    item.campaign.id === campaign.id ||
                                    item.campaign.name === campaign.name
                                );
                                if (inventoryItem) {
                                    const inventoryDrop = inventoryItem.campaign.time_based_drops.find(d => d.id === drop.id);
                                    inventoryProgress = inventoryDrop?.progress;
                                }
                            }
                            
                            // Use whichever progress source is available
                            const progressToUse = prog || inventoryProgress;
                            
                            return {
                                ...drop,
                                progress: progressToUse || drop.progress // Keep existing or use merged
                            };
                        })
                    };

                    // Include every active campaign (mineable or not). The grid's
                    // "Mineable only / All drops" toggle filters non-mineable ones at
                    // render (see filteredGames), so flipping it never reloads.
                    const game = getOrCreateGame(campaign.game_id, campaign.game_name, campaign.image_url);
                    game.active_campaigns.push(campaignWithProgress);
                    game.total_active_drops += campaign.time_based_drops.length;

                    // Update game stats based on merged progress
                    campaignWithProgress.time_based_drops.forEach(drop => {
                        const prog = progressData?.find(p => p.drop_id === drop.id) || drop.progress;
                        if (prog && !prog.is_claimed && prog.current_minutes_watched >= prog.required_minutes_watched) {
                            game.has_claimable = true;
                        }
                        if (prog && !prog.is_claimed && prog.current_minutes_watched > 0) {
                            game.drops_in_progress++;
                        }
                    });
                });
            }

            // Process Inventory Items
            if (inventoryData?.items) {
                inventoryData.items.forEach(item => {
                    let gameId = item.campaign.game_id;
                    const gameName = item.campaign.game_name || "Unknown Game";
                    if (!gameId) gameId = `generated-${gameName.toLowerCase().replace(/\s+/g, '-')}`;

                    const game = getOrCreateGame(gameId, gameName, item.campaign.image_url);
                    game.inventory_items.push(item);
                    game.total_claimed += item.claimed_drops;
                });
            }

            // Get the current automation game name (case-insensitive). Prefer the
            // freshly-read live status (set by the plugin bridge and held in the
            // store while the overlay is closed) over the stale component state.
            const activeAutomationStatus = liveStatus || dropProgress;
            const progressGameName = activeAutomationStatus?.current_drop?.game_name?.toLowerCase() ||
                activeAutomationStatus?.current_channel?.game_name?.toLowerCase();

            // Drops the user has genuinely EARNED, from unambiguous sources only: the
            // permanent gameEventDrops list + any inventory drop explicitly is_claimed.
            // Match by benefit id, plus benefit NAME for badges only (a held badge
            // can't be re-earned; a consumable reissued under a new campaign with the
            // same name and new ids can be). NOTE: NOT claimed-by-index or "100%
            // watched" here; those over-matched in-progress drops as earned.
            const ownedBenefitIds = new Set<string>((inventoryData?.completed_drops || []).map(d => d.id));
            const ownedBenefitNames = new Set<string>(
                (inventoryData?.completed_drops || []).map(d => (d.name || '').toLowerCase().trim()).filter(Boolean)
            );
            const ownedDropIds = new Set<string>();
            inventoryData?.items?.forEach(item => {
                item.campaign.time_based_drops.forEach(drop => {
                    if (drop.progress?.is_claimed === true) {
                        ownedDropIds.add(drop.id);
                        drop.benefit_edges?.forEach(b => {
                            ownedBenefitIds.add(b.id);
                            if (b.name) ownedBenefitNames.add(b.name.toLowerCase().trim());
                        });
                    }
                });
            });
            const ownedByName = (name?: string) => {
                const n = (name || '').toLowerCase().trim();
                return !!n && ownedBenefitNames.has(n) && knownBadgeTitles.has(n);
            };

            // Update active flag and calculate all_drops_claimed for each game
            gamesMap.forEach(game => {
                if (activeAutomationStatus?.active && progressGameName) {
                    game.active = game.name.toLowerCase() === progressGameName;
                }

                // Check if all drops in all active campaigns have been claimed
                if (game.active_campaigns.length > 0 && game.total_active_drops > 0) {
                    let totalDropsInCampaigns = 0;
                    let claimedDropsCount = 0;

                    game.active_campaigns.forEach(campaign => {
                        campaign.time_based_drops.forEach(drop => {
                            totalDropsInCampaigns++;
                            const dp = progressData?.find(p => p.drop_id === drop.id) || drop.progress;
                            // Claimed here, OR already earned elsewhere (by benefit id/name) but
                            // ONLY when this drop isn't itself in progress. A drop with watch-time
                            // is being actively collected and must not be counted as already-earned.
                            const hasCurrentProgress = !!dp && ((dp.current_minutes_watched || 0) > 0 || dp.is_claimed === true);
                            // An inventory drop flagged is_claimed is a proven claim of
                            // THIS drop instance (reissues mint new drop ids), so it
                            // counts even while a stale progress record still carries
                            // watch minutes. Benefit id/name matching stays gated.
                            const owned = dp?.is_claimed === true
                                || ownedDropIds.has(drop.id)
                                || (!hasCurrentProgress && (
                                    drop.benefit_edges?.some(b =>
                                        ownedBenefitIds.has(b.id) || ownedByName(b.name)
                                    ) ?? false
                                ));
                            if (owned) {
                                claimedDropsCount++;
                            }
                        });
                    });

                    // All drops claimed if we have drops and all are claimed.
                    // Prefer embedded per-drop progress (campaign.time_based_drops[].progress) as a fallback,
                    // because `progressData` can be empty before the backend progress map is populated.
                    // Prefer progressData, but also fall back to embedded drop.progress (from CampaignDetails)
                    // to avoid false negatives when progressData isn't populated yet.
                    game.all_drops_claimed = totalDropsInCampaigns > 0 && claimedDropsCount === totalDropsInCampaigns;
                }
            });

            // Only surface games that have at least one campaign with something left to
            // collect. Games whose campaigns are all event-only, or that only appear via
            // earned inventory, have nothing actionable here and live in the Inventory tab.
            setUnifiedGames(Array.from(gamesMap.values()).filter(g => g.active_campaigns.length > 0).sort((a, b) => {
                // Automation games first
                if (a.active !== b.active) return a.active ? -1 : 1;
                // Completed games (all drops claimed) go to bottom
                if (a.all_drops_claimed !== b.all_drops_claimed) return a.all_drops_claimed ? 1 : -1;
                // Games with claimable drops next
                if (a.has_claimable !== b.has_claimable) return a.has_claimable ? -1 : 1;
                // Then by number of active campaigns
                if (a.active_campaigns.length !== b.active_campaigns.length) {
                    return b.active_campaigns.length - a.active_campaigns.length;
                }
                return a.name.localeCompare(b.name);
            }));

        } catch (err) {
            Logger.error('Failed to load unified drops data:', err);
            setError(err instanceof Error ? err.message : String(err));
        } finally {
            setIsLoading(false);
        }
    };

    // ---- Favorite Drops Notification Logic ----
    // Check if any favorited categories have new drops since last session
    const FAVORITE_CAMPAIGNS_CACHE_KEY = 'streamnook_favorite_campaigns_cache';
    
    const checkForNewFavoriteDrops = () => {
        // Get app settings to check if notification is enabled
        const appSettings = useAppStore.getState().settings;
        if (!appSettings.live_notifications?.show_favorite_drops_notifications) {
            Logger.debug('[DropsCenter] Favorite drops notifications disabled');
            return;
        }
        
        // Get current favorite games
        const favoriteGames = dropsSettings?.favorite_games || [];
        if (favoriteGames.length === 0) {
            Logger.debug('[DropsCenter] No favorited games, skipping new drops check');
            return;
        }
        
        Logger.debug('[DropsCenter] Checking for new drops in favorited games:', favoriteGames);
        
        // Get previously cached campaign data
        let cachedData: Record<string, string[]> = {};
        try {
            const cached = localStorage.getItem(FAVORITE_CAMPAIGNS_CACHE_KEY);
            if (cached) {
                cachedData = JSON.parse(cached);
            }
        } catch (e) {
            Logger.warn('[DropsCenter] Failed to parse cached campaign data:', e);
        }
        
        // Build current campaign map for favorited games
        const currentCampaignMap: Record<string, { campaignIds: string[]; gameName: string; boxArt: string }> = {};
        
        unifiedGames.forEach(game => {
            const isFavorite = favoriteGames.some(
                pg => pg.toLowerCase() === game.name.toLowerCase()
            );
            if (!isFavorite) return;
            
            currentCampaignMap[game.name.toLowerCase()] = {
                campaignIds: game.active_campaigns.map(c => c.id),
                gameName: game.name,
                boxArt: game.box_art_url
            };
        });
        
        // Find new campaigns in favorited games
        const newDropNotifications: { gameName: string; boxArt: string; newCount: number; campaignNames: string[] }[] = [];
        
        Object.entries(currentCampaignMap).forEach(([gameKey, data]) => {
            const previousCampaignIds = cachedData[gameKey] || [];
            const newCampaignIds = data.campaignIds.filter(id => !previousCampaignIds.includes(id));
            
            if (newCampaignIds.length > 0) {
                // Find campaign names for the new campaigns
                const game = unifiedGames.find(g => g.name.toLowerCase() === gameKey);
                const campaignNames = game?.active_campaigns
                    .filter(c => newCampaignIds.includes(c.id))
                    .map(c => c.name) || [];
                
                newDropNotifications.push({
                    gameName: data.gameName,
                    boxArt: data.boxArt,
                    newCount: newCampaignIds.length,
                    campaignNames
                });
            }
        });
        
        // Emit notifications for each game with new drops
        newDropNotifications.forEach(({ gameName, boxArt, newCount, campaignNames }) => {
            Logger.debug(`[DropsCenter] New drops available for ${gameName}:`, campaignNames);
            
            emit('new-favorite-drops', {
                game_name: gameName,
                game_image: boxArt,
                new_count: newCount,
                campaign_names: campaignNames
            });
        });
        
        // Update cache with current campaign IDs
        const newCacheData: Record<string, string[]> = {};
        Object.entries(currentCampaignMap).forEach(([gameKey, data]) => {
            newCacheData[gameKey] = data.campaignIds;
        });
        
        try {
            localStorage.setItem(FAVORITE_CAMPAIGNS_CACHE_KEY, JSON.stringify(newCacheData));
        } catch (e) {
            Logger.warn('[DropsCenter] Failed to save campaign cache:', e);
        }
    };

    // ---- Effects ----
    useEffect(() => {
        const init = async () => {
            const auth = await checkAuthentication();
            if (auth) {
                try {
                    // Seed from the bridge-cached live status (a plugin automation),
                    // so reopening mid-automation shows it.
                    const liveStatus = useAppStore.getState().liveDropProgress;
                    if (liveStatus) setDropProgress(liveStatus);
                    const settings = await invoke<DropsSettings>('get_drops_settings');
                    setDropsSettings(settings);
                } catch (e) {
                    Logger.error(e);
                }
                await loadDropsData();
                
                // Check for new drops in favorited categories on startup
                checkForNewFavoriteDrops();
            } else {
                setIsLoading(false);
            }
        };
        init();

        // Listeners
        let isMounted = true;
        let unlistenStatus: (() => void) | undefined;
        let unlistenProgress: (() => void) | undefined;

        const setupListeners = async () => {
            const uStatus = await listen<DropProgressStatus>('drop-progress', (event) => {
                // Single source of truth for automation status. A automation plugin
                // reports through the bridge (PluginAutomationBridge), which emits
                // into this same event so the native UI lights up identically.
                Logger.debug('[DropsCenter] Automation status update:', event.payload);
                setDropProgress(event.payload);
                useAppStore.getState().setDropProgressActive(event.payload.active);
            });
            if (isMounted) unlistenStatus = uStatus; else uStatus();

            const uProgress = await listen<{ drop_id: string; current_minutes: number; required_minutes: number; timestamp: number; campaign_id?: string; }>('drops-progress-update', (event) => {
                Logger.debug('[DropsCenter] Received drops-progress-update:', event.payload);

                // Update progress state
                setProgress((prev) => {
                    const idx = prev.findIndex(p => p.drop_id === event.payload.drop_id);
                    if (idx >= 0) {
                        // Update existing progress
                        const newProg = [...prev];
                        newProg[idx] = {
                            ...newProg[idx],
                            current_minutes_watched: event.payload.current_minutes,
                            required_minutes_watched: event.payload.required_minutes,
                            last_updated: event.payload.timestamp.toString()
                        };
                        Logger.debug('[DropsCenter] Updated existing progress:', newProg[idx]);
                        return newProg;
                    } else {
                        // Add new progress entry
                        const newEntry: DropProgress = {
                            campaign_id: event.payload.campaign_id || '',
                            drop_id: event.payload.drop_id,
                            current_minutes_watched: event.payload.current_minutes,
                            required_minutes_watched: event.payload.required_minutes,
                            is_claimed: false,
                            last_updated: event.payload.timestamp.toString()
                        };
                        Logger.debug('[DropsCenter] Added new progress entry:', newEntry);
                        return [...prev, newEntry];
                    }
                });

                // Update the displayed drop's minutes IN PLACE when this event is
                // for it. WHICH drop is shown (the one finishing first) is decided
                // by the backend and pushed via 'drop-progress', so we never
                // re-select here. That single source of truth is what stops the
                // percentage from flipping between rewards as their progress events
                // arrive out of order, and it advances to the next reward the instant
                // the current one completes.
                setDropProgress((prev) => {
                    if (!prev || !prev.active || !prev.current_drop) return prev;
                    if (prev.current_drop.drop_id !== event.payload.drop_id) return prev;
                    return {
                        ...prev,
                        current_drop: {
                            ...prev.current_drop,
                            current_minutes: event.payload.current_minutes,
                            required_minutes: event.payload.required_minutes
                        },
                        last_update: event.payload.timestamp.toString()
                    };
                });
            });
            if (isMounted) unlistenProgress = uProgress; else uProgress();
        };
        setupListeners();

        return () => {
            isMounted = false;
            if (unlistenStatus) unlistenStatus();
            if (unlistenProgress) unlistenProgress();
        };
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [addToast]);

    // Update games' active flag when dropProgress changes
    useEffect(() => {
        if (!dropProgress || unifiedGames.length === 0) return;

        const progressGameName = dropProgress?.current_drop?.game_name?.toLowerCase() ||
            dropProgress?.current_channel?.game_name?.toLowerCase();

        Logger.debug('[DropsCenter] Updating active flag. Automation:', dropProgress.active, 'Game:', progressGameName);

        // Detect if automation just started for a new game (to trigger scroll)
        const currentAutomationGame = dropProgress.active && progressGameName ? progressGameName : null;
        const prevAutomationGame = prevAutomationGameRef.current;

        // If a new game started automation (different from previous), scroll to top
        if (currentAutomationGame && currentAutomationGame !== prevAutomationGame) {
            Logger.debug('[DropsCenter] New automation game detected, scrolling to top:', currentAutomationGame);
            // Small delay to allow the list to re-sort first
            setTimeout(() => {
                if (gamesContainerRef.current) {
                    gamesContainerRef.current.scrollTo({
                        top: 0,
                        behavior: 'smooth'
                    });
                }
            }, 100);
        }

        // Update the ref for next comparison
        prevAutomationGameRef.current = currentAutomationGame;

        setUnifiedGames(prevGames => {
            const updated = prevGames.map(game => ({
                ...game,
                active: dropProgress.active && progressGameName
                    ? game.name.toLowerCase() === progressGameName
                    : false
            }));

            // Re-sort with full sorting logic
            return updated.sort((a, b) => {
                // Automation games first
                if (a.active !== b.active) return a.active ? -1 : 1;
                // Completed games (all drops claimed) go to bottom
                if (a.all_drops_claimed !== b.all_drops_claimed) return a.all_drops_claimed ? 1 : -1;
                // Games with claimable drops next
                if (a.has_claimable !== b.has_claimable) return a.has_claimable ? -1 : 1;
                // Then by number of active campaigns
                if (a.active_campaigns.length !== b.active_campaigns.length) {
                    return b.active_campaigns.length - a.active_campaigns.length;
                }
                return a.name.localeCompare(b.name);
            });
        });
    }, [dropProgress, unifiedGames.length]);

    // Notification effect: Detect new campaigns from favorite games
    useEffect(() => {
        if (!dropsSettings || unifiedGames.length === 0) return;
        
        const favoriteGames = dropsSettings.favorite_games || [];
        if (favoriteGames.length === 0) return; // No favorites, skip
        
        // Build current campaign IDs set
        const currentCampaignIds = new Set<string>();
        const newFavoriteCampaigns: { gameName: string; campaignName: string }[] = [];
        
        unifiedGames.forEach(game => {
            const isFavorite = favoriteGames.some(
                pg => pg.toLowerCase() === game.name.toLowerCase()
            );
            
            game.active_campaigns.forEach(campaign => {
                currentCampaignIds.add(campaign.id);
                
                // Check if this is a NEW campaign from a favorite game
                if (isFavorite && !prevCampaignIdsRef.current.has(campaign.id)) {
                    // Only notify if we have previous data (not first load)
                    if (prevCampaignIdsRef.current.size > 0) {
                        newFavoriteCampaigns.push({
                            gameName: game.name,
                            campaignName: campaign.name
                        });
                    }
                }
            });
        });
        
        // Send notifications for new favorite campaigns
        if (newFavoriteCampaigns.length > 0 && dropsSettings.notify_on_drop_available) {
            newFavoriteCampaigns.forEach(({ gameName, campaignName }) => {
                addToast(`New drop for ${gameName}: ${campaignName}`, 'success');
                Logger.debug(`[DropsCenter] New favorite campaign notification: ${gameName} - ${campaignName}`);
            });
        }
        
        // Update the ref for next comparison
        prevCampaignIdsRef.current = currentCampaignIds;
    }, [unifiedGames, dropsSettings, addToast]);

    // ---- Render: Authentication Screen ----
    if (!isAuthenticated) {
        return (
            <div className="relative flex flex-col items-center justify-center h-full overflow-hidden">
                {/* Background blur effect with subtle pattern */}
                <div className="absolute inset-0 bg-gradient-to-br from-background via-backgroundSecondary to-background opacity-90" />
                
                {/* Decorative elements - faded gift icons scattered */}
                <div className="absolute inset-0 overflow-hidden pointer-events-none">
                    <Gift className="absolute top-[10%] left-[15%] w-12 h-12 text-accent/5 rotate-12" />
                    <Gift className="absolute top-[25%] right-[20%] w-8 h-8 text-accent/5 -rotate-6" />
                    <Gift className="absolute bottom-[30%] left-[25%] w-10 h-10 text-accent/5 rotate-[-15deg]" />
                    <Gift className="absolute bottom-[15%] right-[15%] w-14 h-14 text-accent/5 rotate-6" />
                    <Gift className="absolute top-[60%] left-[10%] w-6 h-6 text-accent/5 rotate-45" />
                    <Gift className="absolute top-[40%] right-[10%] w-16 h-16 text-accent/5 -rotate-12" />
                </div>

                {/* Main content card */}
                <div className="relative z-10 animate-in fade-in zoom-in-95 duration-500">
                    <div className="glass-panel border border-accent/20 rounded-2xl p-10 max-w-lg mx-4 text-center shadow-2xl shadow-accent/10">
                        {/* Icon with glow effect */}
                        <div className="flex justify-center mb-6">
                            <div className="relative">
                                <div className="absolute inset-0 bg-accent/30 rounded-full blur-xl animate-pulse" />
                                <div className="relative p-5 bg-gradient-to-br from-accent/20 to-accent/5 rounded-full border border-accent/30">
                                    <Gift className="w-14 h-14 text-accent" />
                                </div>
                            </div>
                        </div>

                        {/* Title & Description */}
                        <div className="space-y-3 mb-8">
                            <h2 className="text-3xl font-bold text-textPrimary">Drops Center</h2>
                            <p className="text-textSecondary text-base leading-relaxed">
                                Connect your Twitch account to unlock automatic drop automation, campaign tracking, and reward collection.
                            </p>
                        </div>

                        {/* Features preview - what users get */}
                        <div className="grid grid-cols-3 gap-4 mb-8 py-4 border-y border-borderLight/50">
                            <div className="text-center">
                                <div className="text-accent font-semibold text-lg">Auto</div>
                                <div className="text-textSecondary text-xs">Earning</div>
                            </div>
                            <div className="text-center border-x border-borderLight/50">
                                <div className="text-accent font-semibold text-lg">Track</div>
                                <div className="text-textSecondary text-xs">Progress</div>
                            </div>
                            <div className="text-center">
                                <div className="text-accent font-semibold text-lg">Claim</div>
                                <div className="text-textSecondary text-xs">Rewards</div>
                            </div>
                        </div>

                        {/* Device Code Display (when authenticating) */}
                        {isAuthenticating && deviceCodeInfo && (
                            <div className="mb-6 p-6 bg-accent/5 rounded-xl border border-accent/30 animate-in fade-in slide-in-from-bottom-2 duration-300">
                                <p className="text-xs text-textSecondary uppercase tracking-wider mb-3">Enter this code on Twitch</p>
                                <div className="text-4xl font-mono font-bold text-accent tracking-[0.3em] py-2 select-all">
                                    {deviceCodeInfo.user_code}
                                </div>
                                <div className="flex items-center justify-center gap-2 mt-4 text-textSecondary text-sm">
                                    <div className="w-2 h-2 bg-accent rounded-full animate-pulse" />
                                    <span>Waiting for authorization...</span>
                                </div>
                            </div>
                        )}

                        {/* Login Button */}
                        {!isAuthenticating && (
                            <button
                                onClick={startDropsLogin}
                                className="w-full px-8 py-4 bg-[#9146FF] hover:bg-[#7c3aed] text-white rounded-xl transition-all duration-200 font-semibold flex items-center justify-center gap-3 shadow-lg shadow-[#9146FF]/25 hover:shadow-[#9146FF]/40 hover:scale-[1.02] active:scale-[0.98]"
                            >
                                <TwitchIcon size={22} />
                                <span className="text-base">Connect with Twitch</span>
                            </button>
                        )}

                        {/* Info note */}
                        <p className="mt-6 text-xs text-textSecondary/70">
                            Uses Twitch's Android app authentication for drop compatibility
                        </p>
                    </div>
                </div>
            </div>
        );
    }

    // ---- Render: Main UI ----
    return (
        <div className="flex flex-col h-full bg-background animate-in fade-in">
            {/* Liquid-glass heart gradients for the favorite hearts on game cards. */}
            <svg width="0" height="0" className="absolute pointer-events-none">
                <defs>
                    <linearGradient id="drops-glass-heart-fill" x1="0" y1="0" x2="0" y2="1">
                        <stop offset="0%" stopColor="rgba(255, 255, 255, 0.4)" />
                        <stop offset="30%" stopColor="rgba(236, 72, 153, 0.2)" />
                        <stop offset="100%" stopColor="rgba(236, 72, 153, 0.6)" />
                    </linearGradient>
                    <linearGradient id="drops-glass-heart-stroke" x1="0" y1="0" x2="0" y2="1">
                        <stop offset="0%" stopColor="rgba(255, 255, 255, 0.8)" />
                        <stop offset="100%" stopColor="rgba(255, 255, 255, 0.1)" />
                    </linearGradient>
                </defs>
            </svg>

            {/* Header with Tabs */}
            <div className="flex items-center justify-center gap-2 px-4 py-3 bg-backgroundSecondary border-b border-borderSubtle shrink-0 relative z-20">
                {/* Tab Navigation - Framer Motion Sliding Highlight Style (Centered) */}
                <LayoutGroup>
                <div className="flex items-center glass-panel px-1.5 py-1 rounded-xl">
                    <button
                        onClick={() => setActiveTab('games')}
                        className={`group relative flex items-center gap-2 px-3 py-1.5 text-sm font-medium rounded-lg transition-all duration-300 whitespace-nowrap ${activeTab === 'games'
                            ? 'text-white'
                            : 'text-textSecondary hover:text-textPrimary'
                            }`}
                    >
                        {activeTab === 'games' && (
                            <motion.div
                                layoutId="dropsTabHighlight"
                                className="absolute inset-0 glass-button-static rounded-lg"
                                transition={{ type: "spring", stiffness: 350, damping: 30 }}
                            />
                        )}
                        <span className={`relative z-10 flex items-center gap-2 transition-all duration-300 ${activeTab !== 'games' ? 'group-hover:drop-shadow-[0_2px_4px_rgba(0,0,0,0.8)]' : ''}`}>
                            <MonitorPlay size={16} />
                            <span>Campaigns</span>
                        </span>
                    </button>
                    <button
                        onClick={() => setActiveTab('inventory')}
                        className={`group relative flex items-center gap-2 px-3 py-1.5 text-sm font-medium rounded-lg transition-all duration-300 whitespace-nowrap ${activeTab === 'inventory'
                            ? 'text-white'
                            : 'text-textSecondary hover:text-textPrimary'
                            }`}
                    >
                        {activeTab === 'inventory' && (
                            <motion.div
                                layoutId="dropsTabHighlight"
                                className="absolute inset-0 glass-button-static rounded-lg"
                                transition={{ type: "spring", stiffness: 350, damping: 30 }}
                            />
                        )}
                        <span className={`relative z-10 flex items-center gap-2 transition-all duration-300 ${activeTab !== 'inventory' ? 'group-hover:drop-shadow-[0_2px_4px_rgba(0,0,0,0.8)]' : ''}`}>
                            <Package size={16} />
                            <span>Inventory</span>
                        </span>
                    </button>
                    <button
                        onClick={() => setActiveTab('stats')}
                        className={`group relative flex items-center gap-2 px-3 py-1.5 text-sm font-medium rounded-lg transition-all duration-300 whitespace-nowrap ${activeTab === 'stats'
                            ? 'text-white'
                            : 'text-textSecondary hover:text-textPrimary'
                            }`}
                    >
                        {activeTab === 'stats' && (
                            <motion.div
                                layoutId="dropsTabHighlight"
                                className="absolute inset-0 glass-button-static rounded-lg"
                                transition={{ type: "spring", stiffness: 350, damping: 30 }}
                            />
                        )}
                        <span className={`relative z-10 flex items-center gap-2 transition-all duration-300 ${activeTab !== 'stats' ? 'group-hover:drop-shadow-[0_2px_4px_rgba(0,0,0,0.8)]' : ''}`}>
                            <BarChart3 size={16} />
                            <span>Stats</span>
                        </span>
                    </button>
                    {settingsSlots.length > 0 && (
                    <button
                        onClick={() => setActiveTab('settings')}
                        className={`group relative flex items-center gap-2 px-3 py-1.5 text-sm font-medium rounded-lg transition-all duration-300 whitespace-nowrap ${activeTab === 'settings'
                            ? 'text-white'
                            : 'text-textSecondary hover:text-textPrimary'
                            }`}
                    >
                        {activeTab === 'settings' && (
                            <motion.div
                                layoutId="dropsTabHighlight"
                                className="absolute inset-0 glass-button-static rounded-lg"
                                transition={{ type: "spring", stiffness: 350, damping: 30 }}
                            />
                        )}
                        <span className={`relative z-10 flex items-center gap-2 transition-all duration-300 ${activeTab !== 'settings' ? 'group-hover:drop-shadow-[0_2px_4px_rgba(0,0,0,0.8)]' : ''}`}>
                            <SlidersHorizontal size={16} />
                            <span>Settings</span>
                        </span>
                    </button>
                    )}
                </div>
                </LayoutGroup>

                {/* Account-level logout stays in the tab header (tab-independent);
                    the games browsing controls live in their own toolbar row below,
                    so a descriptive filter label never crowds the centered tabs. */}
                <div className="absolute right-4 flex items-center gap-3">
                    <Tooltip content="Logout from Drops (Android Client)" side="bottom">
                        <button
                            className="px-3 py-1.5 text-xs font-medium rounded-lg glass-panel text-textSecondary hover:text-red-400 hover:bg-red-500/10 hover:border-red-500/30 border border-transparent transition-all"
                            onClick={handleDropsLogout}
                        >
                            Logout
                        </button>
                    </Tooltip>
                </div>
            </div>

            {/* Games browsing toolbar: filter + sort on the left, search on the
                right. Full-width row so labels never fight the tabs for space. */}
            {activeTab === 'games' && (
                <div className="flex items-center justify-between gap-3 px-4 py-2 bg-backgroundSecondary border-b border-borderSubtle shrink-0 relative z-10">
                    <div className="flex items-center gap-3">
                        <SegmentedSelect
                            value={showAllDrops ? 'all' : 'mineable'}
                            onChange={(v) => setShowAllDrops(v === 'all')}
                            options={[
                                { value: 'mineable', label: 'Watch to earn' },
                                { value: 'all', label: 'All drops' },
                            ]}
                        />
                        <Dropdown
                            value={sortMode}
                            onChange={setSortMode}
                            triggerPrefix="Sort"
                            align="left"
                            ariaLabel="Sort games"
                            leadingIcon={<ArrowDownUp size={13} />}
                            options={[
                                { value: 'recommended', label: 'Recommended' },
                                { value: 'newest', label: 'Newest' },
                                { value: 'oldest', label: 'Oldest' },
                            ]}
                        />
                    </div>
                    <div className="relative">
                        <input
                            type="text"
                            placeholder="Search games..."
                            value={searchTerm}
                            onChange={(e) => setSearchTerm(e.target.value)}
                            className="glass-input pl-8 pr-4 py-1.5 text-sm w-48 focus:w-64 transition-all focus:outline-none"
                        />
                        <Search size={14} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-textSecondary" />
                    </div>
                </div>
            )}

            {/* Content Area */}
            <div className="flex-1 overflow-hidden relative">
                {/* Loading State */}
                {isLoading && (
                    <div className="h-full flex items-center justify-center">
                        <LoadingWidget useFunnyMessages={false} message="Loading drops & inventory..." />
                    </div>
                )}

                {/* Games Tab */}
                {!isLoading && activeTab === 'games' && (
                    <div ref={gamesContainerRef} className="h-full overflow-y-auto p-4 custom-scrollbar">
                        {/* Empty State */}
                        {filteredGames.length === 0 && (
                            <div className="flex items-center justify-center h-full">
                                <div className="text-center glass-panel p-8 max-w-sm">
                                    <Gift size={48} className="mx-auto text-textSecondary opacity-40 mb-4" />
                                    <h3 className="text-lg font-bold text-textPrimary mb-2">
                                        {searchTerm ? 'No Games Found' : 'No Drops Available'}
                                    </h3>
                                    <p className="text-sm text-textSecondary">
                                        {searchTerm
                                            ? `No games match "${searchTerm}"`
                                            : 'There are no active drop campaigns right now. Check back later!'
                                        }
                                    </p>
                                </div>
                            </div>
                        )}

                        {/* Game Cards Grid - responsive layout that scales with window size.
                            Fully-collected games are split out of the active grid into
                            their own collapsible section below (same idiom as the
                            Inventory tab's Completed Drops section). */}
                        {filteredGames.length > 0 && (() => {
                            const activeGames = filteredGames.filter(g => !g.all_drops_claimed);
                            const completedGames = filteredGames.filter(g => g.all_drops_claimed);
                            const gridClass = "grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 2xl:grid-cols-7 3xl:grid-cols-8 gap-3 sm:gap-4";
                            const renderGameCard = (game: UnifiedGame) => (
                                <GameCard
                                    key={game.id}
                                    game={game}
                                    allGames={unifiedGames}
                                    progress={progress}
                                    dropProgress={dropProgress}
                                    isSelected={selectedGame?.id === game.id}
                                    isFavorite={(dropsSettings?.favorite_games || []).some(
                                        pg => pg.toLowerCase() === game.name.toLowerCase()
                                    )}
                                    onClick={() => handleGameSelect(selectedGame?.id === game.id ? null : game)}
                                    onStopAutomation={handleStopAutomation}
                                    onToggleFavorite={handleToggleFavorite}
                                />
                            );
                            return (
                                <>
                                    {activeGames.length > 0 && (
                                        <div className={gridClass}>
                                            {activeGames.map(renderGameCard)}
                                        </div>
                                    )}

                                    {/* Everything collected: a calm caught-up note instead of an empty grid */}
                                    {activeGames.length === 0 && !searchTerm && (
                                        <div className="flex flex-col items-center justify-center text-center py-10">
                                            <div className="p-3 rounded-full bg-success/15 mb-3">
                                                <Check size={28} className="text-success" />
                                            </div>
                                            <h3 className="text-lg font-bold text-textPrimary mb-1">All caught up</h3>
                                            <p className="text-sm text-textSecondary">You have collected every available drop.</p>
                                        </div>
                                    )}

                                    {/* Docked completed drawer. Collapsed it is a compact pill in
                                        the bottom-right corner (always in view, near-zero
                                        footprint); expanded it blooms into the full panel sliding
                                        up from the bottom edge. The sticky wrapper spans the row
                                        but is pointer-transparent so cards behind it stay
                                        clickable. */}
                                    {completedGames.length > 0 && (
                                        <div className={`sticky bottom-0 z-20 pointer-events-none ${activeGames.length > 0 ? 'mt-4' : ''}`}>
                                            <AnimatePresence initial={false} mode="wait">
                                                {!showCompletedGames ? (
                                                    <motion.div
                                                        key="completed-pill"
                                                        initial={{ opacity: 0, y: 10, scale: 0.96 }}
                                                        animate={{ opacity: 1, y: 0, scale: 1 }}
                                                        exit={{ opacity: 0, y: 10, scale: 0.96 }}
                                                        // The dynamic-island grow/shrink spring, so both
                                                        // directions of the morph feel identical.
                                                        transition={{ type: 'spring', stiffness: 360, damping: 32, mass: 0.9, opacity: { duration: 0.15 } }}
                                                        className="flex justify-center pb-2"
                                                    >
                                                        {/* Same glass-badge language as the LIVE badge, in
                                                            success green: enough presence to be seen over
                                                            the card grid, still a pill rather than a bar. */}
                                                        <motion.button
                                                            onClick={() => setShowCompletedGames(true)}
                                                            aria-expanded={false}
                                                            whileHover={{ scale: 1.04 }}
                                                            whileTap={{ scale: 0.97 }}
                                                            transition={{ type: 'spring', stiffness: 360, damping: 32, mass: 0.9 }}
                                                            className="pointer-events-auto flex items-center gap-2 px-4 py-2 rounded-full text-sm font-semibold text-textPrimary"
                                                            style={{
                                                                backgroundColor: 'color-mix(in srgb, var(--color-background) 90%, transparent)',
                                                                backgroundImage: 'linear-gradient(135deg, color-mix(in srgb, var(--color-success) 30%, transparent) 0%, color-mix(in srgb, var(--color-success) 18%, transparent) 50%, color-mix(in srgb, var(--color-success) 30%, transparent) 100%)',
                                                                border: '1px solid color-mix(in srgb, var(--color-success) 55%, transparent)',
                                                                boxShadow: 'inset 0 1px 0 rgba(255,255,255,0.25), inset 0 -1px 0 rgba(0,0,0,0.15), 0 4px 16px rgba(0,0,0,0.45)',
                                                            }}
                                                        >
                                                            <Check size={15} className="text-success" strokeWidth={3} />
                                                            <span>Completed</span>
                                                            <span className="px-1.5 py-0.5 rounded-full bg-success/25 text-success text-xs font-bold tabular-nums leading-none">
                                                                {completedGames.length}
                                                            </span>
                                                            <ChevronUp size={15} className="text-textSecondary" />
                                                        </motion.button>
                                                    </motion.div>
                                                ) : (
                                                    <motion.div
                                                        key="completed-panel"
                                                        initial={{ opacity: 0, y: 24 }}
                                                        animate={{ opacity: 1, y: 0 }}
                                                        exit={{ opacity: 0, y: 24 }}
                                                        // Same dynamic-island spring as the pill, so the
                                                        // expand and collapse mirror each other.
                                                        transition={{ type: 'spring', stiffness: 360, damping: 32, mass: 0.9, opacity: { duration: 0.15 } }}
                                                        className="pointer-events-auto glass-panel border border-success/30 overflow-hidden rounded-lg origin-bottom"
                                                        style={{ backgroundColor: 'color-mix(in srgb, var(--color-background) 92%, transparent)' }}
                                                    >
                                                        <button
                                                            onClick={() => setShowCompletedGames(false)}
                                                            aria-expanded={true}
                                                            className="w-full flex items-center justify-between gap-3 p-3 hover:bg-surface/50 transition-colors"
                                                        >
                                                            <div className="flex items-center gap-3">
                                                                <div className="p-2 rounded-lg bg-success/20 border border-success/30">
                                                                    <Check size={20} className="text-success" />
                                                                </div>
                                                                <div className="text-left">
                                                                    <h3 className="font-bold text-textPrimary">Completed</h3>
                                                                    <p className="text-xs text-textSecondary">Every drop for these games is collected</p>
                                                                </div>
                                                            </div>
                                                            <div className="flex items-center gap-2">
                                                                <span className="px-2 py-1 text-xs font-bold rounded-lg bg-success/20 text-success border border-success/30">
                                                                    {completedGames.length} {completedGames.length === 1 ? 'game' : 'games'}
                                                                </span>
                                                                <ChevronDown size={18} className="text-textSecondary" />
                                                            </div>
                                                        </button>
                                                        <div className="border-t border-success/20 bg-background/50 p-3 max-h-[55vh] overflow-y-auto custom-scrollbar">
                                                            <div className={gridClass}>
                                                                {completedGames.map(renderGameCard)}
                                                            </div>
                                                        </div>
                                                    </motion.div>
                                                )}
                                            </AnimatePresence>
                                        </div>
                                    )}
                                </>
                            );
                        })()}

                        {/* Detail Panel */}
                        {selectedGame && (
                            <GameDetailPanel
                                game={selectedGame}
                                allGames={unifiedGames}
                                completedDrops={completedDrops}
                                progress={progress}
                                earnedBadgeTitles={earnedBadgeTitles}
                                knownBadgeTitles={knownBadgeTitles}
                                dropProgress={dropProgress}
                                onClaimDrop={handleClaimDrop}
                                onWatchChannel={handleStreamClick}

                                isOpen={!!selectedGame}
                                onClose={() => setSelectedGame(null)}
                                onStopAutomation={handleStopAutomation}
                            />
                        )}
                    </div>
                )}

                {/* Stats Tab */}
                {!isLoading && activeTab === 'stats' && (
                    <DropsStatsTab
                        statistics={statistics ? {
                            ...statistics,
                            // Drops Claimed = the account's permanent earned-drops inventory
                            // (completed_drops). Fall back to drops claimed in currently-listed
                            // campaigns only when the permanent inventory is empty.
                            total_drops_claimed: completedDrops.length > 0
                                ? completedDrops.reduce((sum, d) => sum + (d.total_count || 1), 0)
                                : unifiedGames.reduce((sum, game) => sum + game.total_claimed, 0),
                            // In Progress = drops the account is actively working on, from the
                            // freshest inventory snapshot; fall back to the live automation count.
                            drops_in_progress: Math.max(
                                inventoryItems.reduce((sum, item) => sum + item.drops_in_progress, 0),
                                statistics.drops_in_progress
                            ),
                        } : null}
                        dropProgress={dropProgress}
                        onStopAutomation={handleStopAutomation}
                        onStreamClick={handleStreamClick}
                    />
                )}

                {/* Inventory Tab */}
                {!isLoading && activeTab === 'inventory' && (
                    <DropsInventoryTab
                        inventoryItems={inventoryItems}
                        completedDrops={completedDrops}
                        progress={progress}
                        onClaimDrop={handleClaimDrop}
                    />
                )}

                {/* Settings Tab — the automation panel a drops-automation plugin
                    contributes into the drops.settings slot. Full-height + scroll
                    so a contribution that manages its own scroll (h-full) is
                    bounded, and a plain one still scrolls here. */}
                {activeTab === 'settings' && (
                    <div className="h-full overflow-y-auto custom-scrollbar">
                        {settingsSlots.map((c) => (
                            <c.Component key={`${c.pluginId}:${c.id}`} />
                        ))}
                    </div>
                )}

            </div>
        </div>
    );
}
