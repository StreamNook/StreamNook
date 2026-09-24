import { useEffect, useState, useMemo, useRef, useCallback } from 'react';
import { motion, AnimatePresence, LayoutGroup } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../stores/AppStore';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Search, Gift, MonitorPlay, BarChart3, Package, ArrowDownUp, SlidersHorizontal, Check, ChevronDown, ChevronUp } from 'lucide-react';
import { usePluginUiRegistry, selectSlot } from '../plugins-ui/registry';
import { Dropdown } from './ui/Dropdown';
import { SegmentedSelect } from './settings/_primitives';
import {
    UnifiedGame, DropCampaign, DropProgress, DropsStatistics,
    DropProgressStatus, InventoryItem, CompletedDrop, TwitchStream, DropsOverview
} from '../types';

import LoadingWidget from './LoadingWidget';
import GameCard from './drops/GameCard';
import GameDetailPanel from './drops/GameDetailPanel';
import DropsStatsTab from './drops/DropsStatsTab';
import DropsInventoryTab from './drops/DropsInventoryTab';
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
    const { addToast, setShowDropsOverlay, dropsSearchTerm, setDropsSearchTerm } = useAppStore();
    


    // Channel Picker State

    // Settings State
    const [dropsSettings, setDropsSettings] = useState<DropsSettings | null>(null);

    // Ref for the games container to scroll to top when automation starts
    const gamesContainerRef = useRef<HTMLDivElement>(null);

    // Track the previously automation game to detect when automation starts
    const prevAutomationGameRef = useRef<string | null>(null);

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
        // Which game is being collected is live UI state (the automation status
        // pushed through the bridge), so it is marked here rather than in Rust.
        let games = unifiedGames.map(g => {
            const active = progressGameName !== null && g.name.toLowerCase() === progressGameName;
            return g.active === active ? g : { ...g, active };
        });

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
                // A game's release recency (Rust's `release_ms`) is its newest
                // active campaign's start.
                const diff = b.release_ms - a.release_ms; // newest-first baseline
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

    // Take the Drops model Rust built (services/drops_overview.rs). The open
    // detail panel holds a snapshot of one game, so it is re-picked by id.
    const applyOverview = (overview: DropsOverview) => {
        setProgress(overview.progress);
        if (overview.statistics) setStatistics(overview.statistics);
        setInventoryItems(overview.inventory_items);
        setCompletedDrops(overview.completed_drops);
        setEarnedBadgeTitles(new Set(overview.earned_badge_titles));
        setKnownBadgeTitles(new Set(overview.known_badge_titles));
        setUnifiedGames(overview.games);
        setSelectedGame(prev => (prev ? overview.games.find(g => g.id === prev.id) ?? prev : prev));
    };

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

            const url = await invoke<string>('start_drops_login');

            // Bound to the active account's web profile (Rust), so it reuses the
            // main login's twitch.tv session - authorize only, no re-login.
            await invoke('open_drops_login_window', { url });
        } catch (err) {
            Logger.error('Failed to start drops login:', err);
            setError(err instanceof Error ? err.message : String(err));
            setIsAuthenticating(false);
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
            setProgress(prev => prev.map(p => (p.drop_id === dropId ? { ...p, is_claimed: true } : p)));

            // A claim only moves a reward into your inventory, so the model is
            // rebuilt from the CACHED campaigns: re-fetching them would reset the
            // backend's live progress map and snap the title-bar progress back.
            applyOverview(await invoke<DropsOverview>('get_drops_overview', { reuseCampaigns: true }));
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
            // Fresh inventory, rebuilt into the model without re-fetching campaigns.
            applyOverview(await invoke<DropsOverview>('get_drops_overview', { reuseCampaigns: true }));
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

            // Rust loads campaigns first (that load refreshes its live progress
            // map), then statistics, inventory and progress, and joins them. It
            // also announces new campaigns in favourite games.
            const overview = await invoke<DropsOverview>('get_drops_overview', { reuseCampaigns: false });

            // Seed automation status from the bridge-cached live status: a plugin
            // powering automation reports through the bridge into the store and keeps
            // it there even while this overlay is closed, so a reopened overlay
            // immediately shows what is being collected.
            const liveStatus = useAppStore.getState().liveDropProgress;
            if (liveStatus) {
                setDropProgress(liveStatus);
            }
            applyOverview(overview);
        } catch (err) {
            Logger.error('Failed to load unified drops data:', err);
            setError(err instanceof Error ? err.message : String(err));
        } finally {
            setIsLoading(false);
        }
    };

    // The overlay reports completion, so the outcome arrives as an event rather
    // than as the resolution of the call that started it.
    useEffect(() => {
        const uns: Array<() => void> = [];
        listen('drops-login-complete', () => {
            setIsAuthenticating(false);
            setIsAuthenticated(true);
            setError(null);
            addToast('Drops login successful!', 'success');
            void loadDropsData();
        }).then((u) => uns.push(u));
        listen<string>('drops-login-error', (e) => {
            setIsAuthenticating(false);
            Logger.error('Failed to complete drops login:', e.payload);
            setError(e.payload);
        }).then((u) => uns.push(u));
        return () => uns.forEach((u) => u());
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);

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
    }, [dropProgress, unifiedGames.length]);

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

                        {/* Authorization in progress */}
                        {isAuthenticating && (
                            <div className="mb-6 p-6 bg-accent/5 rounded-xl border border-accent/30 animate-in fade-in slide-in-from-bottom-2 duration-300">
                                <p className="text-sm text-textSecondary">
                                    Approve the request on Twitch to turn on drops and channel points.
                                </p>
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
