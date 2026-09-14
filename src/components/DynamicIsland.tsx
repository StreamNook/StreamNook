import { useState, useEffect, useLayoutEffect, useRef, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { Bell, Radio, MessageCircle, ChevronRight, User, Download, Gift, Award, Check, CheckCheck, Info, CheckCircle2, AlertTriangle, XCircle } from 'lucide-react';
import { X, SpeakerHigh, SpeakerSlash } from 'phosphor-react';
import { listen, emit } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../stores/AppStore';
import { Logger } from '../utils/logger';
import { deriveBadgeStatus, formatBadgeDateInfo } from '../utils/badgeWindow';
import { playSound, type SoundId } from '../utils/notificationSound';
import { liveActivityText } from '../utils/liveActivity';
import { Tooltip } from './ui/Tooltip';
import type {
    DynamicIslandNotification,
    LiveNotificationData,
    WhisperNotificationData,
    UpdateNotificationData,
    DropsNotificationData,
    ChannelPointsNotificationData,
    BadgeNotificationData,
    SystemNotificationData,
} from '../types';

const MAX_NOTIFICATIONS = 20;
const CACHE_KEY = 'streamnook_notifications';
const CACHE_EXPIRY_DAYS = 7;
// How long a "Test" button dummy notification lingers in the notification
// center before it auto-removes, so previews don't pile up with real ones.
// Kept >= the live preview hold (8s) so the entry outlasts its own preview.
const TEST_NOTIFICATION_TTL_MS = 8000;

/** How long the same channel is suppressed after a go-live announcement.
 *  Module scope, not component state: the listener must not re-arm on every
 *  render, and there is only ever one Dynamic Island. */
const GO_LIVE_DEDUPE_MS = 5 * 60_000;
const recentGoLive = new Map<string, number>();

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
}

interface WhisperFromBackend {
    from_user_id: string;
    from_user_login: string;
    from_user_name: string;
    to_user_id: string;
    to_user_login: string;
    to_user_name: string;
    whisper_id: string;
    text: string;
}

interface BundleUpdateStatus {
    update_available: boolean;
    current_version: string;
    latest_version: string;
    download_url: string | null;
    bundle_name: string | null;
    download_size: string | null;
}

interface DropClaimedEvent {
    drop_name: string;
    game_name: string;
    benefit_name?: string;
    benefit_image_url?: string;
}

interface ChannelPointsEarnedEvent {
    channel_id: string | null;
    channel_login: string | null;
    channel_display_name: string | null;
    points: number;
    reason: string;
    balance: number;
}

// Clustering state for channel points (batching rapid events)
interface ClusteredChannelPoints {
    totalPoints: number;
    events: Array<{
        points: number;
        reason: string;
        channel_name: string | null;
        timestamp: number;
    }>;
    lastUpdate: number;
    lastBalance?: number; // Track the most recent balance
}

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
const getPreviewText = (n: DynamicIslandNotification): string => {
    switch (n.type) {
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
            const base = `+${d.points_earned.toLocaleString()} Channel Points`;
            // channel_name can hold a reason summary like "+20 (watching)" rather
            // than a real channel; only append it when it's a real channel.
            const isReasonSummary = !!d.channel_name && d.channel_name.includes('(');
            return d.channel_name && !isReasonSummary ? `${base} • ${d.channel_name}` : base;
        }
        case 'badge':
            return (n.data as BadgeNotificationData).badge_name;
        case 'system':
            return (n.data as SystemNotificationData).message;
        default:
            return '';
    }
};

// The leading glyph for the collapsed preview: profile picture for live/whisper
// (with an icon fallback), a themed icon for everything else.
const renderPreviewIcon = (n: DynamicIslandNotification): React.ReactNode => {
    switch (n.type) {
        case 'live': {
            const d = n.data as LiveNotificationData;
            return d.streamer_avatar
                ? <img src={d.streamer_avatar} alt="" className="w-4 h-4 rounded-full object-cover flex-shrink-0" />
                : <div className="w-1.5 h-1.5 bg-red-500 rounded-full flex-shrink-0" />;
        }
        case 'whisper': {
            const d = n.data as WhisperNotificationData;
            return d.profile_image_url
                ? <img src={d.profile_image_url} alt="" className="w-4 h-4 rounded-full object-cover flex-shrink-0" />
                : <MessageCircle size={11} className="text-purple-400 flex-shrink-0" />;
        }
        case 'update':
            return <Download size={11} className="text-yellow-400 flex-shrink-0" />;
        case 'drops':
            return <Gift size={11} className="text-green-400 flex-shrink-0" />;
        case 'channel_points':
            return (
                <svg width="11" height="11" viewBox="0 0 24 24" className="text-orange-400 flex-shrink-0" fill="currentColor">
                    <path d="M12 5v2a5 5 0 0 1 5 5h2a7 7 0 0 0-7-7Z"></path>
                    <path fillRule="evenodd" d="M1 12C1 5.925 5.925 1 12 1s11 4.925 11 11-4.925 11-11 11S1 18.075 1 12Zm11 9a9 9 0 1 1 0-18 9 9 0 0 1 0 18Z" clipRule="evenodd"></path>
                </svg>
            );
        case 'badge':
            return <Award size={11} className="text-cyan-400 flex-shrink-0" />;
        case 'system': {
            const lvl = (n.data as SystemNotificationData).level;
            if (lvl === 'success') return <CheckCircle2 size={11} className="text-green-400 flex-shrink-0" />;
            if (lvl === 'error') return <XCircle size={11} className="text-red-400 flex-shrink-0" />;
            if (lvl === 'warning') return <AlertTriangle size={11} className="text-yellow-400 flex-shrink-0" />;
            return <Info size={11} className="text-accent flex-shrink-0" />;
        }
        default:
            return null;
    }
};

const DynamicIsland = () => {
    const [isExpanded, setIsExpanded] = useState(false);
    const [notifications, setNotifications] = useState<DynamicIslandNotification[]>(() => loadCachedNotifications());
    const [hasUnread, setHasUnread] = useState(false);
    const [latestNotification, setLatestNotification] = useState<DynamicIslandNotification | null>(null);
    const [showPreview, setShowPreview] = useState(false);
    const [windowSize, setWindowSize] = useState({ width: window.innerWidth, height: window.innerHeight });
    // Measured width of the current preview's content, so the pill grows to fit
    // the whole message instead of truncating (see the useLayoutEffect below).
    const [previewWidth, setPreviewWidth] = useState(200);
    const previewTextRef = useRef<HTMLSpanElement>(null);
    const islandRef = useRef<HTMLDivElement>(null);
    const previewTimeoutRef = useRef<NodeJS.Timeout | null>(null);
    const updateCheckIntervalRef = useRef<NodeJS.Timeout | null>(null);
    
    // Track current time for "X ago" formatting - updated every 30 seconds
    // This avoids calling Date.now() during render (impure function)
    const [now, setNow] = useState(() => Date.now());
    useEffect(() => {
        const interval = setInterval(() => setNow(Date.now()), 30000);
        return () => clearInterval(interval);
    }, []);

    // Actions are stable, so read them without subscribing; only the two state
    // fields drive re-renders now (this was a whole-store subscription).
    const { startStream, openWhisperWithUser, openSettings, addToast, setShowDropsOverlay, setShowBadgesOverlay, setUpdateInfo } = useAppStore.getState();
    const settings = useAppStore((s) => s.settings);
    const isSettingsOpen = useAppStore((s) => s.isSettingsOpen);

    const soundEnabled = settings.live_notifications?.play_sound ?? true;
    const notificationsEnabled = settings.live_notifications?.enabled ?? true;
    const showLiveNotifications = settings.live_notifications?.show_live_notifications ?? true;
    const showFavoriteLiveNotifications = settings.live_notifications?.show_favorite_live_notifications ?? true;
    const showWhisperNotifications = settings.live_notifications?.show_whisper_notifications ?? true;
    const showUpdateNotifications = settings.live_notifications?.show_update_notifications ?? true;
    const showDropsNotifications = settings.live_notifications?.show_drops_notifications ?? true;
    const showFavoriteDropsNotifications = settings.live_notifications?.show_favorite_drops_notifications ?? true;
    const showChannelPointsNotifications = settings.live_notifications?.show_channel_points_notifications ?? true;
    const showBadgeNotifications = settings.live_notifications?.show_badge_notifications ?? true;
    const useDynamicIsland = settings.live_notifications?.use_dynamic_island ?? true;
    const useToast = settings.live_notifications?.use_toast ?? true;
    const quickUpdateOnToast = settings.live_notifications?.quick_update_on_toast ?? false;

    // Save notifications to cache whenever they change
    useEffect(() => {
        saveCachedNotifications(notifications);
    }, [notifications]);

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
        playSound((settings.live_notifications?.sound_type as SoundId | undefined) ?? 'boop');
    }, [settings.live_notifications?.sound_type]);

    // Send native Windows desktop notification (disabled - plugin not installed)
    const sendNativeNotification = useCallback(async (_title: string, _body: string) => {
        // Native notifications are disabled - the tauri-plugin-notification is not installed
        // To re-enable, add the plugin to Cargo.toml and re-import the functions
    }, []);

    // Add notification
    const addNotification = useCallback((notification: DynamicIslandNotification) => {
        setNotifications(prev => {
            const newNotifications = [notification, ...prev].slice(0, MAX_NOTIFICATIONS);
            return newNotifications;
        });
        setHasUnread(true);
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

    // Mirror action-feedback toasts (fired from anywhere via AppStore.addToast)
    // into the notification center: these are notifications too, so they leave a
    // record even when the toast surface is muted. The store already gated the
    // emit on the island toggles; we re-check here so a mid-flight settings
    // change is honored. These are non-interactive log entries (clicking just
    // marks them read) and play no sound, since the action itself was the user's
    // own context.
    useEffect(() => {
        const unlisten = listen<{ text: string; level?: SystemNotificationData['level'] }>('action-notification', (event) => {
            if (!notificationsEnabled || !useDynamicIsland) return;
            const { text, level } = event.payload;
            if (!text) return;
            addNotification({
                id: `system-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
                type: 'system',
                timestamp: Date.now(),
                read: false,
                data: { title: '', message: text, level } as SystemNotificationData,
            });
        });
        return () => { unlisten.then((fn) => fn()); };
    }, [addNotification, notificationsEnabled, useDynamicIsland]);

    // Check for updates periodically
    const checkForUpdates = useCallback(async () => {
        try {
            const status = await invoke('check_for_bundle_update') as BundleUpdateStatus;
            setUpdateInfo(
                status.update_available
                    ? { current_version: status.current_version, latest_version: status.latest_version }
                    : null
            );

            // Stop here if notification surfaces are disabled. The passive
            // title-bar indicator still surfaces via setUpdateInfo above.
            if (!notificationsEnabled || !showUpdateNotifications) return;

            if (status.update_available) {
                // Check if we already have an update notification for this version
                const existingUpdateNotification = notifications.find(
                    n => n.type === 'update' &&
                        (n.data as UpdateNotificationData).latest_version === status.latest_version
                );

                if (!existingUpdateNotification) {
                    // Add to Dynamic Island if enabled
                    if (useDynamicIsland) {
                        const notification: DynamicIslandNotification = {
                            id: `update-${status.latest_version}-${Date.now()}`,
                            type: 'update',
                            timestamp: Date.now(),
                            read: false,
                            data: {
                                current_version: status.current_version,
                                latest_version: status.latest_version,
                                has_update: true,
                            } as UpdateNotificationData,
                        };

                        addNotification(notification);
                    }

                    // No toast for available updates. The title-bar cog morphs
                    // into a green download button when setUpdateInfo above runs,
                    // which is a less noisy passive signal that the user can act on
                    // at their own pace.

                    if (soundEnabled) {
                        playNotificationSound();
                    }

                    // Send native notification for updates
                    sendNativeNotification(
                        'Update Available',
                        `StreamNook v${status.latest_version} is ready to download`
                    );
                }
            }
        } catch (error) {
            Logger.warn('Could not check for updates:', error);
        }
    }, [notificationsEnabled, showUpdateNotifications, notifications, addNotification, soundEnabled, playNotificationSound, useDynamicIsland, useToast, quickUpdateOnToast, addToast, openSettings, sendNativeNotification, setUpdateInfo]);

    // Check for updates on mount and periodically (every 30 minutes)
    useEffect(() => {
        const initialTimeout = setTimeout(() => {
            checkForUpdates();
        }, 5000);

        updateCheckIntervalRef.current = setInterval(() => {
            checkForUpdates();
        }, 30 * 60 * 1000);

        return () => {
            clearTimeout(initialTimeout);
            if (updateCheckIntervalRef.current) {
                clearInterval(updateCheckIntervalRef.current);
            }
        };
    }, [checkForUpdates]);

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

            // A channel you both follow AND favorite is announced by BOTH
            // watchers, so without this it pops twice. Suppressing a repeat of
            // the same channel for a few minutes also damps a stream that drops
            // and comes straight back, which used to announce itself again.
            // Test notifications are exempt: the point of the button is that it
            // fires every time it is pressed.
            if (!data.is_test) {
                const now = Date.now();
                const key = data.streamer_login.toLowerCase();
                const last = recentGoLive.get(key);
                if (last !== undefined && now - last < GO_LIVE_DEDUPE_MS) return;
                recentGoLive.set(key, now);
                // Bounded: without a sweep this grows for the life of the session.
                for (const [k, t] of recentGoLive) {
                    if (now - t >= GO_LIVE_DEDUPE_MS) recentGoLive.delete(k);
                }
            }

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
                    const testId = `live-test-${Date.now()}`;
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
                const notification: DynamicIslandNotification = {
                    id: `live-${Date.now()}-${data.streamer_login}`,
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
                    } as LiveNotificationData,
                };

                addNotification(notification);
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

            // Send native notification (note: backend also sends one, but this ensures frontend settings are respected)
            sendNativeNotification(
                `${data.streamer_name} is now live!`,
                liveActivityText(data.game_name) || data.stream_title || 'Streaming now'
            );
        });

        return () => {
            unlisten.then((fn) => fn());
        };
    }, [addNotification, notificationsEnabled, showLiveNotifications, showFavoriteLiveNotifications, useDynamicIsland, useToast, addToast, startStream, soundEnabled, playNotificationSound, sendNativeNotification]);

    // Listen for whisper notifications
    useEffect(() => {
        const unlisten = listen<WhisperFromBackend>('whisper-received', async (event) => {
            // Check if notifications are enabled
            if (!notificationsEnabled || !showWhisperNotifications) return;

            const data = event.payload;

            // Get profile image for the sender
            let profileImageUrl: string | undefined;
            try {
                const userInfo = await invoke<{ profile_image_url?: string }>('get_user_by_id', { userId: data.from_user_id });
                profileImageUrl = userInfo.profile_image_url;
            } catch {
                // Ignore error, profile image is optional
            }

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

                if (soundEnabled) {
                    playNotificationSound();
                }
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

            // Send native notification for whispers
            sendNativeNotification(
                `Whisper from ${data.from_user_name}`,
                data.text.length > 100 ? `${data.text.substring(0, 100)}...` : data.text
            );
            // Note: Whisper conversation storage is handled by WhispersWidget
        });

        return () => {
            unlisten.then((fn) => fn());
        };
    }, [addNotification, playNotificationSound, notificationsEnabled, showWhisperNotifications, soundEnabled, useDynamicIsland, useToast, addToast, openWhisperWithUser, sendNativeNotification]);

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

                if (soundEnabled) {
                    playNotificationSound();
                }
            }

            // Show toast if enabled
            if (useToast) {
                addToast(
                    `Drop claimed: ${data.drop_name} (${data.game_name})`,
                    'success',
                    {
                        label: 'View',
                        onClick: () => setShowDropsOverlay(true),
                    },
                    { skipIsland: true }
                );
            }

            // Send native notification for drops
            sendNativeNotification(
                'Drop Claimed',
                `${data.drop_name} - ${data.game_name}`
            );
        });

        return () => {
            unlisten.then((fn) => fn());
        };
    }, [addNotification, notificationsEnabled, showDropsNotifications, useDynamicIsland, useToast, addToast, soundEnabled, playNotificationSound, setShowDropsOverlay, sendNativeNotification]);

    // Listen for channel points earned notifications with clustering
    // Ref to track clustered channel points
    const channelPointsClusterRef = useRef<ClusteredChannelPoints>({
        totalPoints: 0,
        events: [],
        lastUpdate: 0,
    });
    const channelPointsTimeoutRef = useRef<NodeJS.Timeout | null>(null);

    // Function to format reason codes into human-readable text
    const formatReasonCode = (reason: string): string => {
        const reasonMap: Record<string, string> = {
            'WATCH': 'watching',
            'WATCH_STREAK': 'watch streak',
            'CLAIM': 'bonus claim',
            'AUTOMATION': 'automation',
            'RAID': 'raid',
            'PREDICTION': 'prediction',
            'BITS': 'bits',
            'SUB': 'subscription',
            'GIFT_SUB': 'gift sub',
        };
        return reasonMap[reason.toUpperCase()] || reason.toLowerCase();
    };

    // Function to flush clustered channel points as a single notification
    const flushChannelPointsCluster = useCallback(() => {
        const cluster = channelPointsClusterRef.current;
        if (cluster.events.length === 0) return;

        // Create a summary of the clustered events
        const totalPoints = cluster.totalPoints;

        // Get unique channel names
        const uniqueChannels = [...new Set(cluster.events.map(e => e.channel_name).filter(Boolean))];

        // Group events by channel for display
        const channelPoints: Record<string, number> = {};
        cluster.events.forEach(e => {
            const channel = e.channel_name || 'Unknown';
            channelPoints[channel] = (channelPoints[channel] || 0) + e.points;
        });

        // Per-channel earned breakdown ("ninja +50, pokimane +82"), highest
        // first and capped so the line stays short. This names the actual
        // streamers the points came from instead of a bare "N channels" count.
        const MAX_CHANNELS_SHOWN = 3;
        const channelBreakdown = Object.entries(channelPoints)
            .filter(([name]) => name && name !== 'Unknown')
            .sort((a, b) => b[1] - a[1]);
        const channelListDisplay = channelBreakdown.length === 0
            ? null
            : channelBreakdown.length <= MAX_CHANNELS_SHOWN
                ? channelBreakdown.map(([name, pts]) => `${name} +${pts.toLocaleString()}`).join(', ')
                : `${channelBreakdown.slice(0, MAX_CHANNELS_SHOWN).map(([name, pts]) => `${name} +${pts.toLocaleString()}`).join(', ')} +${channelBreakdown.length - MAX_CHANNELS_SHOWN} more`;

        // Group events by reason for display
        const reasonCounts: Record<string, number> = {};
        cluster.events.forEach(e => {
            const reason = formatReasonCode(e.reason);
            reasonCounts[reason] = (reasonCounts[reason] || 0) + e.points;
        });

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
                    total_points: uniqueChannels.length > 1 ? undefined : cluster.lastBalance,
                    // Mark if this is a reason summary (not a real channel name)
                    is_reason_summary: !channelNameDisplay,
                } as ChannelPointsNotificationData,
            };

            addNotification(notification);

            if (soundEnabled) {
                playNotificationSound();
            }
        }

        // Show toast if enabled - show rich formatted version
        if (useToast) {
            const toastContent = (
                <div className="flex items-center gap-3 w-full">
                    {/* Channel Points Icon */}
                    <div className="w-10 h-10 rounded-full bg-orange-500/20 flex items-center justify-center flex-shrink-0">
                        <svg width="20" height="20" viewBox="0 0 24 24" className="text-orange-400" fill="currentColor">
                            <path d="M12 5v2a5 5 0 0 1 5 5h2a7 7 0 0 0-7-7Z"></path>
                            <path fillRule="evenodd" d="M1 12C1 5.925 5.925 1 12 1s11 4.925 11 11-4.925 11-11 11S1 18.075 1 12Zm11 9a9 9 0 1 1 0-18 9 9 0 0 1 0 18Z" clipRule="evenodd"></path>
                        </svg>
                    </div>

                    {/* Text Content */}
                    <div className="flex-1 min-w-0">
                        <div className="text-base font-semibold text-textPrimary">
                            +{totalPoints.toLocaleString()} Channel Points
                        </div>
                        <div className="text-xs text-textSecondary">
                            {uniqueChannels.length === 1 && uniqueChannels[0] ? (
                                // Single channel - show channel name, reason, and new balance
                                cluster.lastBalance
                                    ? `${uniqueChannels[0]} • ${formatReasonCode(cluster.events[0]?.reason || 'watch')} • ${cluster.lastBalance.toLocaleString()} points`
                                    : `${uniqueChannels[0]} • ${formatReasonCode(cluster.events[0]?.reason || 'watch')}`
                            ) : uniqueChannels.length > 1 ? (
                                // Multiple channels - name the streamers + each earn
                                channelListDisplay ?? `From ${uniqueChannels.length} channels`
                            ) : (
                                // No channel info - show reason and balance if available
                                cluster.lastBalance
                                    ? `${formatReasonCode(cluster.events[0]?.reason || 'watch')} • ${cluster.lastBalance.toLocaleString()} points`
                                    : formatReasonCode(cluster.events[0]?.reason || 'watch')
                            )}
                        </div>
                    </div>
                </div>
            );

            addToast(toastContent, 'channel_points');
        }

        // Send native notification for channel points
        const channelInfo = uniqueChannels.length === 1 && uniqueChannels[0]
            ? uniqueChannels[0]
            : uniqueChannels.length > 1
                ? (channelListDisplay ?? `${uniqueChannels.length} channels`)
                : formatReasonCode(cluster.events[0]?.reason || 'watch');
        sendNativeNotification(
            `+${totalPoints.toLocaleString()} Channel Points`,
            channelInfo
        );

        // Reset the cluster
        channelPointsClusterRef.current = {
            totalPoints: 0,
            events: [],
            lastUpdate: 0,
        };
    }, [useDynamicIsland, useToast, addNotification, addToast, soundEnabled, playNotificationSound, sendNativeNotification]);

    useEffect(() => {
        const unlisten = listen<ChannelPointsEarnedEvent>('channel-points-earned', (event) => {
            if (!notificationsEnabled || !showChannelPointsNotifications) return;

            const data = event.payload;

            // Skip if no points (shouldn't happen, but safety check)
            if (!data.points || data.points <= 0) return;

            // Get channel name from available sources
            const channelName = data.channel_display_name || data.channel_login || null;

            // Add to the cluster
            channelPointsClusterRef.current.totalPoints += data.points;
            channelPointsClusterRef.current.events.push({
                points: data.points,
                reason: data.reason || 'watch',
                channel_name: channelName,
                timestamp: Date.now(),
            });
            channelPointsClusterRef.current.lastUpdate = Date.now();

            // Store the latest balance
            if (data.balance) {
                channelPointsClusterRef.current.lastBalance = data.balance;
            }

            // Clear any existing timeout
            if (channelPointsTimeoutRef.current) {
                clearTimeout(channelPointsTimeoutRef.current);
            }

            // Set a new timeout to flush the cluster after 3 seconds of no new events
            // This batches rapid-fire notifications together
            channelPointsTimeoutRef.current = setTimeout(() => {
                flushChannelPointsCluster();
            }, 3000);
        });

        return () => {
            unlisten.then((fn) => fn());
            // Flush any remaining clustered notifications on unmount
            if (channelPointsTimeoutRef.current) {
                clearTimeout(channelPointsTimeoutRef.current);
            }
            if (channelPointsClusterRef.current.events.length > 0) {
                flushChannelPointsCluster();
            }
        };
    }, [notificationsEnabled, showChannelPointsNotifications, flushChannelPointsCluster]);

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
                        } as BadgeNotificationData,
                    };

                    addNotification(notification);
                }

                // The pushed status is a snapshot of when the relay sent it, so
                // a badge queued before its window opened would announce itself
                // as "Coming soon" after it had already gone live. Prefer the
                // window when the relay gave us one.
                // `date_info` is passed as the copy to parse so a badge with no
                // enrichment still classifies off the stamps in its own window.
                const derived = deriveBadgeStatus(badge.date_info, badge.enrichment);
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

                // Send native notification for badges
                const nativeStatusText = effectiveStatus === 'new' ? 'New badge available' :
                    effectiveStatus === 'available' ? 'Badge available now' : 'Badge coming soon';
                sendNativeNotification(
                    badge.badge_name,
                    `${nativeStatusText}${when ? ` - ${when}` : ''}`
                );

                if (soundEnabled) {
                    playNotificationSound();
                }
            });
        });

        return () => {
            unlisten.then((fn) => fn());
        };
    }, [notificationsEnabled, showBadgeNotifications, useDynamicIsland, useToast, addNotification, addToast, soundEnabled, playNotificationSound, setShowBadgesOverlay, sendNativeNotification]);

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

            // Send native notification
            sendNativeNotification(
                `New drops for ${data.game_name}!`,
                data.new_count === 1 
                    ? data.campaign_names[0] 
                    : `${data.new_count} new campaigns available`
            );

            if (soundEnabled) {
                playNotificationSound();
            }
        });

        return () => {
            unlisten.then((fn) => fn());
        };
    }, [notificationsEnabled, showFavoriteDropsNotifications, useDynamicIsland, useToast, addNotification, addToast, soundEnabled, playNotificationSound, setShowDropsOverlay, sendNativeNotification]);

    // Click outside to close
    useEffect(() => {
        const handleClickOutside = (event: MouseEvent) => {
            if (islandRef.current && !islandRef.current.contains(event.target as Node)) {
                setIsExpanded(false);
            }
        };

        if (isExpanded) {
            document.addEventListener('mousedown', handleClickOutside);
        }

        return () => {
            document.removeEventListener('mousedown', handleClickOutside);
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
        } else if (notification.type === 'update') {
            // Open settings to the Updates tab
            openSettings("What's New");
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

    // Clear all notifications
    const clearAllNotifications = () => {
        setNotifications([]);
        setHasUnread(false);
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
        setHasUnread(false);
    };

    // Get unread count
    const unreadCount = notifications.filter(n => !n.read).length;

    // Update hasUnread when notifications change
    useEffect(() => {
        setHasUnread(unreadCount > 0);
    }, [unreadCount]);

    // Format time ago - uses 'now' state (updated every 30s) to avoid impure function calls during render
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
        if (showPreview && latestNotification) {
            // Fit the pill to the current preview's content (measured in the
            // useLayoutEffect above) so the whole message shows instead of
            // truncating. The pill is center-anchored (left-1/2 +
            // -translate-x-1/2), so it expands outward from the middle on both
            // sides, then contracts back once the preview auto-hides.
            return previewWidth;
        }
        // Idle: a fixed compact pill. The quiet unread dot fits without widening.
        return 72;
    };

    return (
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
                className={`fixed left-1/2 -translate-x-1/2 top-1.5 ${isSettingsOpen ? 'z-[55]' : 'z-50'}`}
                style={{ WebkitAppRegion: 'no-drag' } as React.CSSProperties}
            >
                <motion.div
                    // No `layout` here on purpose: width/height are animated
                    // explicitly below, and the parent centers with translateX(-50%).
                    // Framer's layout projection fighting that recentering is what
                    // caused the left/right jitter as the pill settled back.
                    initial={false}
                    animate={{
                        width: isExpanded ? expandedWidth : getCollapsedWidth(),
                        // Collapsed pill sits centered in the title bar. The bar is
                        // h-[40px] (less a 1px bottom border); container at top-1.5 (6px)
                        // + a 28px pill leaves an even gap above and below.
                        height: isExpanded ? Math.min(maxHeight, 64 + notifications.length * itemHeight) : 28,
                    }}
                    transition={{
                        // Softer than a snappy popup so the pill flows open and
                        // contracts cleanly. This spring drives the dynamic-island
                        // grow/shrink as notifications come and go.
                        type: 'spring',
                        stiffness: 360,
                        damping: 32,
                        mass: 0.9,
                    }}
                    onClick={() => {
                        if (!isExpanded) {
                            setIsExpanded(true);
                            setShowPreview(false);
                            if (previewTimeoutRef.current) {
                                clearTimeout(previewTimeoutRef.current);
                            }
                        }
                    }}
                    className="dynamic-island overflow-hidden cursor-pointer"
                    style={{
                        backgroundColor: '#000000',
                        borderRadius: isExpanded ? 20 : 14,
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
                                        </span>
                                    </motion.div>
                                ) : (
                                    // Default state: just a quiet accent dot when there are
                                    // unread notifications, otherwise an empty black pill.
                                    // The count itself lives in the expanded header.
                                    <div className="flex items-center justify-center w-full">
                                        {hasUnread ? (
                                            <span className="block w-1.5 h-1.5 rounded-full bg-accent" />
                                        ) : null}
                                    </div>
                                )}
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
                                {/* Header */}
                                <div
                                    className="relative flex items-center px-5 py-4"
                                    style={{
                                        borderBottom: '1px solid rgba(255, 255, 255, 0.1)',
                                    }}
                                >
                                    {/* Invisible close button in the middle third */}
                                    <Tooltip content="Click to close" side="bottom">
                                        <div
                                            className="absolute left-1/3 right-1/3 top-0 bottom-0 cursor-pointer z-10"
                                            onClick={(e) => {
                                                e.stopPropagation();
                                                setIsExpanded(false);
                                            }}
                                        />
                                    </Tooltip>
                                    {/* Sound icon on the left */}
                                    <div className="flex-shrink-0">
                                        {soundEnabled ? (
                                            <SpeakerHigh size={16} className="text-white/40" />
                                        ) : (
                                            <SpeakerSlash size={16} className="text-white/30" />
                                        )}
                                    </div>
                                    {/* Centered notifications text and count */}
                                    <div className="flex-1 flex items-center justify-center gap-2">
                                        <span className="text-white font-semibold text-base">Notifications</span>
                                        {unreadCount > 0 && (
                                            <span
                                                className="text-[10px] font-bold px-1.5 py-0.5 rounded-full text-black"
                                                style={{
                                                    backgroundColor: 'var(--color-accent)',
                                                }}
                                            >
                                                {unreadCount}
                                            </span>
                                        )}
                                    </div>
                                    {/* Empty spacer on the right to balance the sound icon */}
                                    <div className="flex-shrink-0 w-4" />
                                </div>

                                {/* Notifications List */}
                                <div className="flex-1 overflow-y-auto scrollbar-thin">
                                    {notifications.length === 0 ? (
                                        <div className="flex flex-col items-center justify-center py-12 text-white/40">
                                            <Bell size={28} className="mb-3" />
                                            <span className="text-sm">No notifications</span>
                                        </div>
                                    ) : (
                                        <div className="p-3 space-y-2">
                                            {notifications.map((notification) => (
                                                <motion.div
                                                    key={notification.id}
                                                    layout
                                                    initial={{ opacity: 0, y: -10 }}
                                                    animate={{ opacity: 1, y: 0 }}
                                                    exit={{ opacity: 0, x: -100 }}
                                                    onClick={() => handleNotificationClick(notification)}
                                                    className={`
                                                        flex items-center gap-4 p-3 rounded-xl cursor-pointer
                                                        ${notification.read ? 'bg-white/5' : 'bg-white/10'}
                                                        hover:bg-white/15 transition-colors group
                                                    `}
                                                >
                                                    {notification.type === 'live' ? (
                                                        <>
                                                            {/* Live notification */}
                                                            <div className="relative flex-shrink-0">
                                                                {(notification.data as LiveNotificationData).streamer_avatar ? (
                                                                    <img
                                                                        src={(notification.data as LiveNotificationData).streamer_avatar}
                                                                        alt=""
                                                                        className="w-12 h-12 rounded-full object-cover"
                                                                    />
                                                                ) : (
                                                                    <div className="w-12 h-12 rounded-full bg-white/10 flex items-center justify-center">
                                                                        <User size={18} className="text-white/50" />
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
                                                                    <img
                                                                        src={(notification.data as WhisperNotificationData).profile_image_url}
                                                                        alt=""
                                                                        className="w-12 h-12 rounded-full object-cover"
                                                                    />
                                                                ) : (
                                                                    <div className="w-12 h-12 rounded-full bg-purple-500/20 flex items-center justify-center">
                                                                        <MessageCircle size={18} className="text-purple-400" />
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
                                                    ) : notification.type === 'update' ? (
                                                        <>
                                                            {/* Update notification */}
                                                            <div className="relative flex-shrink-0">
                                                                <div className="w-12 h-12 rounded-full bg-yellow-500/20 flex items-center justify-center">
                                                                    <Download size={18} className="text-yellow-400" />
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
                                                                        className="w-12 h-12 rounded-lg object-cover"
                                                                    />
                                                                ) : (
                                                                    <div className="w-12 h-12 rounded-full bg-green-500/20 flex items-center justify-center">
                                                                        <Gift size={18} className="text-green-400" />
                                                                    </div>
                                                                )}
                                                            </div>
                                                            <div className="flex-1 min-w-0">
                                                                <div className="flex items-center gap-2">
                                                                    <Gift size={14} className="text-green-400 flex-shrink-0" />
                                                                    <span className="text-white text-sm font-semibold truncate">
                                                                        Drop Claimed
                                                                    </span>
                                                                </div>
                                                                <p className="text-white/50 text-sm truncate mt-0.5">
                                                                    {(notification.data as DropsNotificationData).drop_name} ({(notification.data as DropsNotificationData).game_name})
                                                                </p>
                                                            </div>
                                                        </>
                                                    ) : notification.type === 'channel_points' ? (
                                                        <>
                                                            {/* Channel Points notification */}
                                                            <div className="relative flex-shrink-0">
                                                                <div className="w-12 h-12 rounded-full bg-orange-500/20 flex items-center justify-center">
                                                                    <svg width="18" height="18" viewBox="0 0 24 24" className="text-orange-400" fill="currentColor">
                                                                        <path d="M12 5v2a5 5 0 0 1 5 5h2a7 7 0 0 0-7-7Z"></path>
                                                                        <path fillRule="evenodd" d="M1 12C1 5.925 5.925 1 12 1s11 4.925 11 11-4.925 11-11 11S1 18.075 1 12Zm11 9a9 9 0 1 1 0-18 9 9 0 0 1 0 18Z" clipRule="evenodd"></path>
                                                                    </svg>
                                                                </div>
                                                            </div>
                                                            <div className="flex-1 min-w-0">
                                                                <div className="flex items-center gap-2">
                                                                    <svg width="14" height="14" viewBox="0 0 24 24" className="text-orange-400 flex-shrink-0" fill="currentColor">
                                                                        <path d="M12 5v2a5 5 0 0 1 5 5h2a7 7 0 0 0-7-7Z"></path>
                                                                        <path fillRule="evenodd" d="M1 12C1 5.925 5.925 1 12 1s11 4.925 11 11-4.925 11-11 11S1 18.075 1 12Zm11 9a9 9 0 1 1 0-18 9 9 0 0 1 0 18Z" clipRule="evenodd"></path>
                                                                    </svg>
                                                                    <span className="text-white text-sm font-semibold truncate">
                                                                        Channel Points +{(notification.data as ChannelPointsNotificationData).points_earned.toLocaleString()}
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
                                                                            return `${channelName}: ${totalPoints.toLocaleString()} points`;
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
                                                                        className="w-12 h-12 rounded-lg object-cover"
                                                                    />
                                                                ) : (
                                                                    <div className="w-12 h-12 rounded-full bg-cyan-500/20 flex items-center justify-center">
                                                                        <Award size={18} className="text-cyan-400" />
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
                                                                        const derived = deriveBadgeStatus(data.date_info, data.enrichment);
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
                                                                const Icon = d.level === 'success' ? CheckCircle2
                                                                    : d.level === 'error' ? XCircle
                                                                    : d.level === 'warning' ? AlertTriangle
                                                                    : Info;
                                                                const color = d.level === 'success' ? 'text-green-400'
                                                                    : d.level === 'error' ? 'text-red-400'
                                                                    : d.level === 'warning' ? 'text-yellow-400'
                                                                    : 'text-accent';
                                                                return (
                                                                    <>
                                                                        <div className="relative flex-shrink-0">
                                                                            <div className="w-12 h-12 rounded-full bg-white/10 flex items-center justify-center">
                                                                                <Icon size={18} className={color} />
                                                                            </div>
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

                                                    <div className="flex items-center gap-1.5 flex-shrink-0">
                                                        <span className="text-white/30 text-xs">
                                                            {formatTimeAgo(notification.timestamp)}
                                                        </span>
                                                        {/* Mark as read button - only show for unread notifications */}
                                                        {!notification.read && (
                                                            <Tooltip content="Mark as read" side="top">
                                                                <button
                                                                    onClick={(e) => markAsRead(notification.id, e)}
                                                                    className="opacity-0 group-hover:opacity-100 text-white/30 hover:text-green-400 transition-all p-0.5"
                                                                >
                                                                    <Check size={14} />
                                                                </button>
                                                            </Tooltip>
                                                        )}
                                                        <Tooltip content="Remove notification" side="top">
                                                            <button
                                                                onClick={(e) => clearNotification(notification.id, e)}
                                                                className="opacity-0 group-hover:opacity-100 text-white/30 hover:text-white transition-all p-0.5"
                                                            >
                                                                <X size={14} />
                                                            </button>
                                                        </Tooltip>
                                                        <ChevronRight size={14} className="text-white/30" />
                                                    </div>
                                                </motion.div>
                                            ))}
                                            {/* Footer Actions */}
                                            <div className="flex gap-2 mt-2">
                                                {/* Mark All as Read Button - only show if there are unread */}
                                                {unreadCount > 0 && (
                                                    <button
                                                        onClick={(e) => {
                                                            e.stopPropagation();
                                                            markAllAsRead();
                                                        }}
                                                        className="flex-1 py-2 text-white/40 hover:text-green-400 text-xs transition-colors text-center rounded-lg hover:bg-white/5 flex items-center justify-center gap-1.5"
                                                    >
                                                        <CheckCheck size={12} />
                                                        Mark all read
                                                    </button>
                                                )}
                                                {/* Clear All Button */}
                                                <button
                                                    onClick={(e) => {
                                                        e.stopPropagation();
                                                        clearAllNotifications();
                                                    }}
                                                    className={`${unreadCount > 0 ? 'flex-1' : 'w-full'} py-2 text-white/40 hover:text-white/70 text-xs transition-colors text-center rounded-lg hover:bg-white/5`}
                                                >
                                                    Clear all
                                                </button>
                                            </div>
                                        </div>
                                    )}
                                </div>
                            </motion.div>
                        )}
                    </AnimatePresence>
                </motion.div>
            </div>
    );
};

export default DynamicIsland;
