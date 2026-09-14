import { Logger } from '../utils/logger';
// IVR API Service for fetching additional Twitch user data
// API Documentation: https://api.ivr.fi

interface IVRUserData {
    id: string;
    login: string;
    displayName: string;
    createdAt: string;
    roles: {
        isAffiliate: boolean;
        isPartner: boolean;
        isSiteAdmin: boolean;
        isStaff: boolean;
    };
    profileImageUrl: string;
    banned: boolean;
    banReason: string | null;
    chatColor: string;
    emotePrefix: string | null;
    followers: number;
}

interface IVRSubageData {
    user: {
        id: string;
        login: string;
        displayName: string;
    };
    channel: {
        id: string;
        login: string;
        displayName: string;
    };
    statusHidden: boolean;
    followedAt: string | null;
    subscriber: boolean;
    subscriptionTier: number | null;
    cumulative: {
        months: number;
    } | null;
    streak: {
        months: number;
    } | null;
    gift: boolean;
    founder: boolean;
    giftCount: number | null;
    meta: {
        subMonths: number;
        subStreak: number;
        subGiftCount: number;
    } | null;
}

export interface IVRProfileData {
    createdAt: string | null;
    followingSince: string | null;
    statusHidden: boolean;
    isSubscribed: boolean;
    subStreak: number | null;
    subCumulative: number | null;
    isFounder: boolean;
    isMod: boolean;
    modSince: string | null;
    isVip: boolean;
    vipSince: string | null;
    isLoading: boolean;
    error: string | null;
}

// Cache for IVR API results
interface CacheEntry<T> {
    data: T;
    timestamp: number;
}

const CACHE_DURATION = 5 * 60 * 1000; // 5 minutes cache
const userDataCache = new Map<string, CacheEntry<IVRUserData | null>>();
const subageCache = new Map<string, CacheEntry<IVRSubageData | null>>();

/**
 * Fetches user data from IVR API
 * @param username - The Twitch username to look up
 * @returns User data or null if not found
 */
export async function fetchIVRUserData(username: string): Promise<IVRUserData | null> {
    const cacheKey = username.toLowerCase();
    const cached = userDataCache.get(cacheKey);

    if (cached && Date.now() - cached.timestamp < CACHE_DURATION) {
        Logger.debug('[IVR] Using cached user data for:', username);
        return cached.data;
    }

    try {
        Logger.debug('[IVR] Fetching user data for:', username);
        const response = await fetch(`https://api.ivr.fi/v2/twitch/user?login=${encodeURIComponent(username)}`);

        if (!response.ok) {
            Logger.error('[IVR] API error:', response.status, response.statusText);
            userDataCache.set(cacheKey, { data: null, timestamp: Date.now() });
            return null;
        }

        const data = await response.json();

        // API returns an array, get first result
        if (Array.isArray(data) && data.length > 0) {
            const userData = data[0] as IVRUserData;
            userDataCache.set(cacheKey, { data: userData, timestamp: Date.now() });
            return userData;
        }

        userDataCache.set(cacheKey, { data: null, timestamp: Date.now() });
        return null;
    } catch (error) {
        Logger.error('[IVR] Failed to fetch user data:', error);
        return null;
    }
}

/**
 * Fetches subage/follow data from IVR API
 * @param username - The Twitch username to look up
 * @param channel - The channel to check following status for
 * @returns Subage data or null if not found
 */
export async function fetchIVRSubage(username: string, channel: string): Promise<IVRSubageData | null> {
    const cacheKey = `${username.toLowerCase()}:${channel.toLowerCase()}`;
    const cached = subageCache.get(cacheKey);

    if (cached && Date.now() - cached.timestamp < CACHE_DURATION) {
        Logger.debug('[IVR] Using cached subage data for:', username, 'in', channel);
        return cached.data;
    }

    try {
        Logger.debug('[IVR] Fetching subage data for:', username, 'in', channel);
        const response = await fetch(
            `https://api.ivr.fi/v2/twitch/subage/${encodeURIComponent(username)}/${encodeURIComponent(channel)}`
        );

        if (!response.ok) {
            Logger.error('[IVR] Subage API error:', response.status, response.statusText);
            subageCache.set(cacheKey, { data: null, timestamp: Date.now() });
            return null;
        }

        const data = await response.json() as IVRSubageData;
        subageCache.set(cacheKey, { data, timestamp: Date.now() });
        return data;
    } catch (error) {
        Logger.error('[IVR] Failed to fetch subage data:', error);
        return null;
    }
}

/**
 * Formats a date string into a human-readable format
 * @param dateString - ISO date string
 * @param includeRelative - Whether to include relative time (e.g., "5 years ago")
 * @returns Formatted date string
 */
export function formatIVRDate(dateString: string, includeRelative: boolean = true): string {
    try {
        const date = new Date(dateString);
        const now = new Date();

        // Format the absolute date
        const absoluteDate = date.toLocaleDateString('en-US', {
            year: 'numeric',
            month: 'long',
            day: 'numeric'
        });

        if (!includeRelative) {
            return absoluteDate;
        }

        // Calculate relative time
        const diffMs = now.getTime() - date.getTime();
        const diffDays = Math.floor(diffMs / (1000 * 60 * 60 * 24));
        const diffMonths = Math.floor(diffDays / 30);
        const diffYears = Math.floor(diffDays / 365);

        let relativeTime: string;
        if (diffYears > 0) {
            relativeTime = diffYears === 1 ? '1 year ago' : `${diffYears} years ago`;
        } else if (diffMonths > 0) {
            relativeTime = diffMonths === 1 ? '1 month ago' : `${diffMonths} months ago`;
        } else if (diffDays > 0) {
            relativeTime = diffDays === 1 ? '1 day ago' : `${diffDays} days ago`;
        } else {
            relativeTime = 'today';
        }

        return `${absoluteDate} (${relativeTime})`;
    } catch (error) {
        Logger.error('[IVR] Failed to format date:', error);
        return dateString;
    }
}

/**
 * Formats subscription tenure with streak and cumulative months
 * @param streak - Current streak months
 * @param cumulative - Total cumulative months
 * @returns Formatted tenure string
 */
export function formatSubTenure(streak: number | null, cumulative: number | null): string {
    if (streak === null && cumulative === null) return '';
    if (streak === null) return `${cumulative} months`;
    if (cumulative === null) return `${streak} months`;

    if (streak === cumulative) {
        return `${streak} ${streak === 1 ? 'month' : 'months'}`;
    }

    return `${streak} ${streak === 1 ? 'month' : 'months'} (${cumulative} cumulative)`;
}

/**
 * Clears the IVR cache
 */
export function clearIVRCache(): void {
    userDataCache.clear();
    subageCache.clear();
    Logger.debug('[IVR] Cache cleared');
}

/**
 * Gets the current cache size
 */
export function getIVRCacheSize(): { users: number; subages: number } {
    return {
        users: userDataCache.size,
        subages: subageCache.size,
    };
}
