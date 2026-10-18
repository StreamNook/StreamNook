import { createClient, RealtimeChannel, SupabaseClient } from '@supabase/supabase-js';
import { invoke } from '@tauri-apps/api/core';
import type { TwitchUser } from '../types';
import { getClientConfig } from './clientConfig';

import { Logger } from '../utils/logger';
import type { Atmosphere } from './atmospheres';
import { DEV_ATMOSPHERES } from './devAtmospheres';
import {
    requestStreamnookArt,
    streamnookArtUrlsIn,
    subscribeStreamnookArt,
    withStreamnookArtCopies,
} from './streamnookArt';
import { DEV_COSMETICS, isDevCosmetic, readDevActiveCosmetic, writeDevActiveCosmetic } from './devCosmetics';
import type { ActiveEquipment, CosmeticSlot } from './cosmetics/types';
// Supabase client singleton
const supabaseUrl = import.meta.env.VITE_SUPABASE_URL;
const supabaseAnonKey = import.meta.env.VITE_SUPABASE_ANON_KEY;

// Only initialize if credentials are provided
const isConfigured = supabaseUrl && supabaseAnonKey &&
    supabaseUrl !== 'your_supabase_url_here' &&
    supabaseAnonKey !== 'your_supabase_anon_key_here';

let supabase: SupabaseClient | null = null;

if (isConfigured) {
    supabase = createClient(supabaseUrl, supabaseAnonKey);
    Logger.debug('[Supabase] Client initialized');
} else {
    Logger.warn('[Supabase] Not configured - analytics features disabled. Add VITE_SUPABASE_URL and VITE_SUPABASE_ANON_KEY to .env');
}

// Presence channel reference
let presenceChannel: RealtimeChannel | null = null;
let currentPresenceKey: string | null = null;
let currentPresencePayload: PresenceState | null = null;
let presenceReady = false;

// Listeners that want to be notified whenever presenceState changes (sync/join/leave).
// Stored as a Set so we can actually remove individual subscribers, unlike before.
const onlineCountSubscribers = new Set<(snapshot: OnlinePresenceSnapshot) => void>();

// Types for presence tracking
interface PresenceState {
    online_at: string;
    user_id?: string;
    display_name?: string;
    app_version?: string;
    /** 'android' or 'desktop'. Rides alongside app_version so any surface that
     *  already reads presence can tell the mobile beta apart from the desktop
     *  client without a schema change - presence payloads are just JSON. */
    platform?: string;
}

/**
 * Rich snapshot of who is currently online via the global presence channel.
 * Deduped by user_id; anonymous keys counted separately so the surfaces can
 * show "N users + M anonymous" if they want.
 */
export interface OnlinePresenceSnapshot {
    authedUserIds: Set<string>;
    anonKeyCount: number;
    /** Total unique participants = authed users + anon sessions. */
    totalUnique: number;
    /** Map of authed user_id -> richest known presence payload. */
    byUserId: Map<string, { display_name?: string; app_version?: string; platform?: string; online_at: string }>;
}

const emptySnapshot = (): OnlinePresenceSnapshot => ({
    authedUserIds: new Set(),
    anonKeyCount: 0,
    totalUnique: 0,
    byUserId: new Map(),
});

const computeSnapshot = (): OnlinePresenceSnapshot => {
    if (!presenceChannel) return emptySnapshot();
    const state = presenceChannel.presenceState<PresenceState>();
    const authedUserIds = new Set<string>();
    const byUserId = new Map<string, { display_name?: string; app_version?: string; platform?: string; online_at: string }>();
    let anonKeyCount = 0;

    for (const key of Object.keys(state)) {
        const presences = state[key];
        if (!Array.isArray(presences) || presences.length === 0) continue;
        // Pick the freshest payload for this key (Supabase usually returns one
        // per key, but be defensive against multi-tab same-key edge cases).
        const fresh = presences.reduce((a, b) =>
            new Date(b.online_at).getTime() > new Date(a.online_at).getTime() ? b : a
        );
        if (fresh.user_id) {
            if (!authedUserIds.has(fresh.user_id)) {
                authedUserIds.add(fresh.user_id);
                byUserId.set(fresh.user_id, {
                    display_name: fresh.display_name,
                    app_version: fresh.app_version,
                    platform: fresh.platform,
                    online_at: fresh.online_at,
                });
            } else {
                // Already counted; keep the freshest payload.
                const existing = byUserId.get(fresh.user_id);
                if (!existing || new Date(fresh.online_at).getTime() > new Date(existing.online_at).getTime()) {
                    byUserId.set(fresh.user_id, {
                        display_name: fresh.display_name,
                        app_version: fresh.app_version,
                        platform: fresh.platform,
                        online_at: fresh.online_at,
                    });
                }
            }
        } else {
            anonKeyCount++;
        }
    }

    return {
        authedUserIds,
        anonKeyCount,
        totalUnique: authedUserIds.size + anonKeyCount,
        byUserId,
    };
};

const notifyOnlineSubscribers = () => {
    if (onlineCountSubscribers.size === 0) return;
    const snap = computeSnapshot();
    for (const cb of onlineCountSubscribers) {
        try { cb(snap); } catch (e) { Logger.error('[Supabase] Online subscriber error:', e); }
    }
};

/**
 * Get the Supabase client instance
 */
export const getSupabaseClient = (): SupabaseClient | null => supabase;

/**
 * Check if Supabase is configured and ready to use
 */
export const isSupabaseConfigured = (): boolean => !!isConfigured;

/**
 * Build a stable presence key. Authenticated users key by `user:<id>` so
 * reconnects land in the same presence slot instead of inflating the count.
 * Anonymous sessions get a random key so unrelated tabs/installs stay distinct.
 */
const buildPresenceKey = (userId?: string): string => {
    if (userId) return `user:${userId}`;
    return `anon:${Date.now()}_${Math.random().toString(36).slice(2, 11)}`;
};

const buildPayload = (userId?: string, displayName?: string, appVersion?: string): PresenceState => ({
    online_at: new Date().toISOString(),
    ...(userId && { user_id: userId }),
    ...(displayName && { display_name: displayName }),
    ...(appVersion && { app_version: appVersion }),
    // Derived here rather than threaded through every caller: it is a property
    // of the running client, not of the call site.
    //
    // `clientPlatform()` rather than `IS_MOBILE ? 'android' : 'desktop'`: that
    // expression labelled an iPhone 'android' while `clientPlatform()` (which
    // feeds `record_client` and `/api/v1/user/sync`) called the same client
    // 'ios', so presence and the persisted row disagreed by construction. One
    // answer now. Same three values as before on every other client, so nothing
    // downstream re-buckets.
    platform: clientPlatform(),
});

const attachPresenceListeners = (channel: RealtimeChannel) => {
    channel
        .on('presence', { event: 'sync' }, () => {
            presenceReady = true;
            notifyOnlineSubscribers();
        })
        .on('presence', { event: 'join' }, () => {
            notifyOnlineSubscribers();
        })
        .on('presence', { event: 'leave' }, () => {
            notifyOnlineSubscribers();
        });
};

/**
 * Track user presence in the app. Safe to call repeatedly — when called with
 * a different identity (e.g. anon -> authed) it tears down the old channel
 * and rejoins with the new key, so the global presence count never carries
 * the old anonymous session forward.
 *
 * @returns Unsubscribe function to clean up presence on unmount
 */
export const trackPresence = async (
    userId?: string,
    displayName?: string,
    appVersion?: string
): Promise<(() => void) | null> => {
    if (!supabase) {
        Logger.debug('[Supabase] Skipping presence tracking - not configured');
        return null;
    }

    try {
        const nextKey = buildPresenceKey(userId);

        // If we are already tracking under the right key, just refresh the payload.
        if (presenceChannel && currentPresenceKey === nextKey) {
            currentPresencePayload = buildPayload(userId, displayName, appVersion);
            await presenceChannel.track(currentPresencePayload);
            return () => disposePresence();
        }

        // Otherwise tear down the old channel and rejoin under the new key.
        if (presenceChannel) {
            try { await presenceChannel.untrack(); } catch { /* best effort */ }
            try { await presenceChannel.unsubscribe(); } catch { /* best effort */ }
            presenceChannel = null;
            presenceReady = false;
        }

        currentPresenceKey = nextKey;
        currentPresencePayload = buildPayload(userId, displayName, appVersion);

        presenceChannel = supabase.channel('global-presence', {
            config: { presence: { key: currentPresenceKey } },
        });

        attachPresenceListeners(presenceChannel);

        presenceChannel.subscribe(async (status) => {
            if (status === 'SUBSCRIBED') {
                if (currentPresencePayload) {
                    await presenceChannel?.track(currentPresencePayload);
                }
                Logger.debug('[Supabase] Presence tracking active as', currentPresenceKey);
                // Push an immediate snapshot so subscribers that registered
                // before SUBSCRIBED resolved get the first count without
                // waiting for another sync event.
                notifyOnlineSubscribers();
            } else if (status === 'CHANNEL_ERROR' || status === 'TIMED_OUT' || status === 'CLOSED') {
                Logger.warn('[Supabase] Presence channel status:', status);
                presenceReady = false;
            }
        });

        return () => disposePresence();
    } catch (error) {
        Logger.error('[Supabase] Failed to track presence:', error);
        return null;
    }
};

const disposePresence = () => {
    if (!presenceChannel) return;
    Logger.debug('[Supabase] Cleaning up presence tracking');
    try { presenceChannel.untrack(); } catch { /* best effort */ }
    try { presenceChannel.unsubscribe(); } catch { /* best effort */ }
    presenceChannel = null;
    currentPresenceKey = null;
    currentPresencePayload = null;
    presenceReady = false;
};

/**
 * Update presence when the user logs in (or out). Re-keys the channel so the
 * authenticated user lands in their stable `user:<id>` slot. Safe to call
 * before trackPresence has finished — it will just defer until then.
 */
export const updatePresence = async (userId: string, displayName: string, appVersion?: string): Promise<void> => {
    if (!supabase) {
        Logger.debug('[Supabase] Skipping presence update - not configured');
        return;
    }

    // Re-key by delegating to trackPresence with the new identity.
    await trackPresence(userId, displayName, appVersion);
};

/**
 * Upsert user data to the users table on login
 * @param user - TwitchUser object
 * @param appVersion - Optional app version
 */
export const upsertUser = async (user: TwitchUser, appVersion?: string): Promise<void> => {
    // Server-authoritative path. The server takes the user id from the bearer
    // token, so this can only ever write the caller's own row.
    const synced = await writeViaApi('/api/v1/user/sync', {
        username: user.login || user.username,
        display_name: user.display_name || user.username,
        avatar_url: user.profile_image_url ?? null,
        app_version: appVersion ?? null,
        platform: clientPlatform(),
    });
    if (synced.handled) {
        // recordClient is folded into the sync endpoint; presence is separate and
        // still ours to send. Sent regardless of whether the row write succeeded:
        // presence and liveness must not disappear just because the sync failed,
        // or a broken cohort goes invisible instead of looking broken.
        await updatePresence(user.user_id, user.display_name || user.username, appVersion);
        return;
    }

    if (!supabase) {
        Logger.debug('[Supabase] Skipping user upsert - not configured');
        return;
    }

    try {
        const payload: any = {
            id: user.user_id,
            username: user.login || user.username,
            display_name: user.display_name || user.username,
            avatar_url: user.profile_image_url || null,
            last_seen: new Date().toISOString(),
        };

        if (appVersion) {
            payload.app_version = appVersion;
        }

        const { error } = await supabase
            .from('users')
            .upsert(payload, {
                onConflict: 'id',
            });

        if (error) {
            // Deliberately does NOT return. The client-record and presence calls
            // below have nothing to do with whether the `users` row was written,
            // and bailing here makes a client that cannot write `users` also stop
            // refreshing last_seen, stop recording which client it is, and never
            // join presence. That cohort then goes INVISIBLE rather than looking
            // broken, which is the worst possible failure for the adoption
            // measurement that gates revoking the anon write.
            Logger.error('[Supabase] Failed to upsert user:', error);
        } else {
            Logger.debug('[Supabase] User upserted:', user.display_name || user.username);
        }

        // Which client they signed in from. Not awaited: it is reporting, and
        // presence is what the user actually notices.
        void recordClient(user.user_id);

        // Also update presence with user info
        await updatePresence(user.user_id, user.display_name || user.username, appVersion);
    } catch (error) {
        Logger.error('[Supabase] Failed to upsert user:', error);
    }
};

/**
 * Which StreamNook client this is.
 *
 * Read off the user agent rather than an OS plugin, matching how the Android
 * port decides its own layout: the Android System WebView always says "Android"
 * in its UA, so this is synchronous and needs no extra capability entry. Shared
 * code, so the Android build reports itself correctly with no separate branch.
 */
const clientPlatform = (): string =>
    typeof navigator !== 'undefined' && /android|iphone|ipad|ipod/i.test(navigator.userAgent)
        ? (/android/i.test(navigator.userAgent) ? 'android' : 'ios')
        : 'desktop';

/**
 * Record which client a member signed in from.
 *
 * Presence answers "who is on Android right now"; this answers "who uses
 * Android". Without a persisted row an offline Android user looks like a
 * desktop one.
 *
 * Fire-and-forget: reporting must never be able to fail a sign-in.
 */
const recordClient = async (twitchUserId: string): Promise<void> => {
    if (!supabase || !twitchUserId) return;
    try {
        const { error } = await supabase.rpc('record_client', {
            p_user_id: twitchUserId,
            p_platform: clientPlatform(),
        });
        if (error) Logger.debug('[Supabase] Could not record client:', error);
    } catch (e) {
        Logger.debug('[Supabase] Could not record client:', e);
    }
};

/**
 * Get the current count of online users via presence. Deduped by user_id so a
 * user logged in on multiple machines counts as one; anonymous sessions each
 * count as one.
 */
export const getOnlineCount = (): number => computeSnapshot().totalUnique;

/**
 * Get the full presence snapshot (authed user_ids, anon count, per-user
 * payloads). Useful for the Settings panel and the dashboard.
 */
export const getOnlinePresenceSnapshot = (): OnlinePresenceSnapshot => computeSnapshot();

/**
 * Subscribe to presence snapshot changes. Works even before `trackPresence`
 * has finished initialising the channel — the callback will fire as soon as
 * the first sync arrives. Returns a real unsubscribe that removes only this
 * listener.
 */
export const subscribeToOnlinePresence = (
    callback: (snap: OnlinePresenceSnapshot) => void
): (() => void) => {
    onlineCountSubscribers.add(callback);
    // Push the current snapshot immediately so the caller always starts with
    // *something* (zeros if the channel hasn't subscribed yet).
    try { callback(computeSnapshot()); } catch (e) { Logger.error('[Supabase] Online subscriber error:', e); }
    return () => { onlineCountSubscribers.delete(callback); };
};

/**
 * Back-compat helper that just wraps subscribeToOnlinePresence and emits the
 * deduped count. Existing call sites keep working.
 */
export const subscribeToOnlineCount = (
    callback: (count: number) => void
): (() => void) => {
    return subscribeToOnlinePresence((snap) => callback(snap.totalUnique));
};

/**
 * True once the presence channel has received its first sync event.
 * Lets the UI distinguish "0 online" from "still connecting".
 */
export const isPresenceReady = (): boolean => presenceReady;

// Type for user data returned from database
export interface SupabaseUser {
    id: string;
    username: string;
    display_name: string;
    avatar_url: string | null;
    last_seen: string;
    created_at: string;
    app_version?: string;
}

// Type for user stats
export interface UserStats {
    user_id: string;
    channel_points_collected: number;
    hours_watched: number;
    messages_sent: number;
    streams_watched: number;
    // How many times other members have opened this member's public profile.
    // Bumped by the `increment_profile_view` RPC; optional so older rows / a
    // pre-migration DB don't break the read.
    profile_views?: number;
    // Twitch Recap from before the member joined, as counted by the server:
    // hours, chats sent, channel points earned in their top channels, and subs
    // gifted. Added to the tracked stats for display; optional so a
    // pre-migration DB does not break the read.
    recap_hours?: number;
    recap_messages?: number;
    recap_points?: number;
    recap_subs_gifted?: number;
    updated_at: string;
}

// Type for user with stats combined
export interface UserWithStats extends SupabaseUser {
    stats?: UserStats;
}

// Type for global stats
export interface GlobalStats {
    total_channel_points: number;
    total_hours_watched: number;
    total_messages_sent: number;
    total_streams_watched: number;
}

// ---------------------------------------------------------------------------
// Stat increment health tracking
// ---------------------------------------------------------------------------
// The dashboard needs to know whether writes are actually landing. Without
// this, a broken RLS policy or missing RPC silently zeros every chart.

export type StatWriteIssue =
    | { kind: 'rpc_missing'; lastSeen: string }
    | { kind: 'rls_denied'; lastSeen: string; detail: string }
    | { kind: 'other'; lastSeen: string; detail: string };

let lastWriteIssue: StatWriteIssue | null = null;
const writeIssueSubscribers = new Set<(issue: StatWriteIssue | null) => void>();

const reportWriteIssue = (issue: StatWriteIssue | null) => {
    lastWriteIssue = issue;
    for (const cb of writeIssueSubscribers) {
        try { cb(issue); } catch (e) { Logger.error('[Supabase] Write-issue subscriber error:', e); }
    }
};

export const getLastWriteIssue = (): StatWriteIssue | null => lastWriteIssue;

export const subscribeToWriteIssues = (cb: (issue: StatWriteIssue | null) => void): (() => void) => {
    writeIssueSubscribers.add(cb);
    try { cb(lastWriteIssue); } catch (e) { Logger.error('[Supabase] Write-issue subscriber error:', e); }
    return () => { writeIssueSubscribers.delete(cb); };
};

/**
 * Increment a user stat. Uses the `increment_user_stat` RPC when available
 * for atomicity; falls back to a manual upsert otherwise. Surfaces RLS /
 * permission failures via the write-issue subscription so the dashboard can
 * show a banner instead of silently zeroing the charts.
 */
export const incrementStat = async (
    userId: string,
    stat: 'channel_points_collected' | 'hours_watched' | 'messages_sent' | 'streams_watched',
    amount: number = 1
): Promise<void> => {
    if (!supabase) {
        Logger.warn('[Supabase] Cannot increment stat - Supabase not configured');
        return;
    }
    if (!userId) {
        Logger.warn('[Supabase] Cannot increment stat - No user ID provided');
        return;
    }

    Logger.debug(`[Supabase] Attempting to increment ${stat} by ${amount} for user ${userId}`);

    try {
        const { error } = await supabase.rpc('increment_user_stat', {
            p_user_id: userId,
            p_stat: stat,
            p_amount: amount
        });

        if (!error) {
            Logger.debug(`[Supabase] Incremented ${stat} by ${amount} for user ${userId}`);
            // Successful write clears the last issue (recovered).
            if (lastWriteIssue) reportWriteIssue(null);
            return;
        }

        // 42883 = function does not exist; PGRST202 = RPC route missing in PostgREST.
        const rpcMissing = error.code === '42883'
            || error.code === 'PGRST202'
            || (typeof error.message === 'string' && error.message.toLowerCase().includes('function'));

        if (rpcMissing) {
            reportWriteIssue({ kind: 'rpc_missing', lastSeen: new Date().toISOString() });
            Logger.debug('[Supabase] RPC missing, using manual upsert fallback');
            await manualIncrementStat(userId, stat, amount);
            return;
        }

        // 42501 = insufficient_privilege (RLS denial).
        if (error.code === '42501') {
            reportWriteIssue({
                kind: 'rls_denied',
                lastSeen: new Date().toISOString(),
                detail: error.message || 'RLS denied',
            });
            Logger.error('[Supabase] Stat increment denied by RLS:', error.message);
            return;
        }

        Logger.error('[Supabase] Failed to increment stat via RPC:', error.message, error.code);
        reportWriteIssue({
            kind: 'other',
            lastSeen: new Date().toISOString(),
            detail: `${error.code || ''} ${error.message || ''}`.trim(),
        });
        await manualIncrementStat(userId, stat, amount);
    } catch (error: any) {
        Logger.error('[Supabase] Exception incrementing stat:', error);
        reportWriteIssue({
            kind: 'other',
            lastSeen: new Date().toISOString(),
            detail: String(error?.message || error),
        });
        try {
            await manualIncrementStat(userId, stat, amount);
        } catch (fallbackError) {
            Logger.error('[Supabase] Manual fallback also failed:', fallbackError);
        }
    }
};

/**
 * Manual stat increment fallback (when RPC not available). Not atomic;
 * concurrent increments can lose writes. The RPC path is preferred.
 */
const manualIncrementStat = async (
    userId: string,
    stat: 'channel_points_collected' | 'hours_watched' | 'messages_sent' | 'streams_watched',
    amount: number
): Promise<void> => {
    if (!supabase) return;

    try {
        const { data: existing, error: selectError } = await supabase
            .from('user_stats')
            .select('*')
            .eq('user_id', userId)
            .maybeSingle();

        if (selectError && selectError.code !== 'PGRST116') {
            if (selectError.code === '42501') {
                reportWriteIssue({
                    kind: 'rls_denied',
                    lastSeen: new Date().toISOString(),
                    detail: selectError.message || 'RLS denied on select',
                });
            }
            throw selectError;
        }

        if (existing) {
            const newValue = ((existing as any)[stat] || 0) + amount;
            const { error } = await supabase
                .from('user_stats')
                .update({ [stat]: newValue, updated_at: new Date().toISOString() })
                .eq('user_id', userId);
            if (error) throw error;
        } else {
            const { error } = await supabase
                .from('user_stats')
                .insert({
                    user_id: userId,
                    [stat]: amount,
                    updated_at: new Date().toISOString(),
                });
            if (error) throw error;
        }

        if (lastWriteIssue) reportWriteIssue(null);
    } catch (error: any) {
        Logger.error('[Supabase] Manual increment failed:', error);
        if (error?.code === '42501') {
            reportWriteIssue({
                kind: 'rls_denied',
                lastSeen: new Date().toISOString(),
                detail: error.message || 'RLS denied',
            });
        } else {
            reportWriteIssue({
                kind: 'other',
                lastSeen: new Date().toISOString(),
                detail: `${error?.code || ''} ${error?.message || ''}`.trim(),
            });
        }
    }
};

/**
 * Get stats for a specific user
 * @param userId - User ID
 * @returns User stats or null
 */
export const getUserStats = async (userId: string): Promise<UserStats | null> => {
    if (!supabase || !userId) {
        return null;
    }

    try {
        const { data, error } = await supabase
            .from('user_stats')
            .select('*')
            .eq('user_id', userId)
            .single();

        if (error && error.code !== 'PGRST116') { // PGRST116 = no rows returned
            Logger.error('[Supabase] Failed to get user stats:', error);
            return null;
        }

        return data as UserStats || null;
    } catch (error) {
        Logger.error('[Supabase] Failed to get user stats:', error);
        return null;
    }
};

// Profiles already counted this app session, so reopening a member's profile in
// the same session doesn't keep inflating their view count (deduped total).
const countedProfileViews = new Set<string>();

/**
 * Record a profile view: bump the VIEWED member's `profile_views` via the
 * `increment_profile_view` RPC (security definer, so a viewer can increment
 * someone else's count). Deduped per session. Returns the new total, or null if
 * it was deduped / the RPC isn't available yet (best-effort, never throws).
 */
export const incrementProfileView = async (userId: string): Promise<number | null> => {
    if (!supabase || !userId) return null;
    if (countedProfileViews.has(userId)) return null; // already counted this session
    countedProfileViews.add(userId);
    try {
        const { data, error } = await supabase.rpc('increment_profile_view', { p_user_id: userId });
        if (error) {
            // Missing column / RPC (pre-migration DB): degrade silently. Drop the
            // session mark so it can retry once the backend exists.
            countedProfileViews.delete(userId);
            Logger.debug('[Supabase] increment_profile_view unavailable:', error.message);
            return null;
        }
        return typeof data === 'number' ? data : null;
    } catch (error) {
        countedProfileViews.delete(userId);
        Logger.debug('[Supabase] incrementProfileView failed:', error);
        return null;
    }
};

/**
 * Read a member's current `profile_views` count without incrementing (used when
 * we shouldn't count: own profile, the live preview, or an already-counted view).
 * Returns null if unavailable.
 */
export const getProfileViews = async (userId: string): Promise<number | null> => {
    if (!supabase || !userId) return null;
    try {
        const { data, error } = await supabase
            .from('user_stats')
            .select('profile_views')
            .eq('user_id', userId)
            .maybeSingle();
        if (error) {
            Logger.debug('[Supabase] getProfileViews failed:', error.message);
            return null;
        }
        const n = (data as { profile_views?: number } | null)?.profile_views;
        return typeof n === 'number' ? n : null;
    } catch (error) {
        Logger.debug('[Supabase] getProfileViews failed:', error);
        return null;
    }
};

/**
 * Earned accolade ids for a member (seasonal, limited, and so on).
 *
 * Returns [] only when Supabase is unconfigured or no user is signed in.
 * A failed READ REJECTS rather than resolving to []: both callers apply the
 * result as the new earned set, so an empty answer on a flaky moment would
 * un-light every medallion until the next window focus. A rejection leaves
 * what is on screen alone; callers catch it.
 */
export const getAccolades = async (userId: string): Promise<string[]> => {
    if (!userId) return [];
    // Rust reads them (services/profile_view.rs) and rejects on a failed read
    // rather than answering an empty set, so lit medallions never blank.
    return invoke<string[]>('get_accolades', { userId });
};

/**
 * Grant an award badge to a user (idempotent). Insert-only; re-granting an
 * already-earned badge is a no-op via ON CONFLICT.
 */
export const grantAccolade = async (userId: string, accoladeId: string): Promise<void> => {
    if (!supabase || !userId || !accoladeId) return;
    try {
        const { error } = await supabase
            .from('user_accolades')
            .upsert(
                { twitch_user_id: userId, accolade_id: accoladeId, earned_at: new Date().toISOString() },
                { onConflict: 'twitch_user_id,accolade_id', ignoreDuplicates: true },
            );
        if (error) Logger.error('[Supabase] Failed to grant award badge:', error.message);
    } catch (error) {
        Logger.error('[Supabase] Failed to grant award badge:', error);
    }
};

/**
 * Claim a reward (event or milestone). The server function (claim_reward,
 * security definer) checks the gate against its own state: an event window
 * against its own clock, or a milestone threshold against the real stored value
 * (e.g. subscriber months, watch hours). A rolled-back clock or a faked client
 * number cannot earn it. It then grants the accolade or cosmetic. `granted` is
 * true only on a fresh earn (idempotent: re-claiming returns ok with
 * granted:false). `reason` carries 'window_closed' / 'not_started' /
 * 'below_threshold' / 'inactive' / 'unknown_reward' on a server reject, or the
 * error code on a transport/RLS failure.
 */
export const claimReward = async (
    userId: string,
    rewardId: string,
    context: Record<string, unknown> = {},
): Promise<{ ok: boolean; granted?: boolean; reason?: string; rewardId?: string }> => {
    if (!supabase || !userId || !rewardId) return { ok: false, reason: 'unconfigured' };
    try {
        const { data, error } = await supabase.rpc('claim_reward', {
            p_twitch_user_id: userId,
            p_reward_id: rewardId,
            p_context: context,
        });
        if (error) {
            // 42501 = RLS denial; 42883/PGRST202 = function not deployed yet.
            Logger.error('[Supabase] claim_reward failed:', error.message, error.code);
            return { ok: false, reason: error.code || 'error' };
        }
        const res = (data ?? {}) as { ok?: boolean; granted?: boolean; reason?: string; reward_id?: string };
        return { ok: !!res.ok, granted: res.granted, reason: res.reason, rewardId: res.reward_id };
    } catch (error) {
        Logger.error('[Supabase] Exception claiming reward:', error);
        return { ok: false, reason: 'exception' };
    }
};

export interface EventReward {
    id: string;
    title: string | null;
    reward_kind: string;
    reward_id: string;
}

/**
 * Active event rewards (the watch-to-earn kind), cached briefly since they rarely
 * change. Lets the generic watch-reward claimer treat new events as config, not a
 * release. Reads the world-readable rewards table; returns the last good result
 * if a refresh fails.
 */
let eventRewardsCache: { at: number; rows: EventReward[] } | null = null;
export const listActiveEventRewards = async (): Promise<EventReward[]> => {
    if (!supabase) return [];
    const now = Date.now();
    if (eventRewardsCache && now - eventRewardsCache.at < 5 * 60_000) return eventRewardsCache.rows;
    try {
        const { data, error } = await supabase
            .from('rewards')
            .select('id, title, reward_kind, reward_id')
            .eq('active', true)
            .eq('gate_kind', 'event');
        if (error) {
            Logger.warn('[Supabase] listActiveEventRewards failed:', error.message);
            return eventRewardsCache?.rows ?? [];
        }
        const rows = (data ?? []) as EventReward[];
        eventRewardsCache = { at: now, rows };
        return rows;
    } catch (error) {
        Logger.warn('[Supabase] listActiveEventRewards exception:', error);
        return eventRewardsCache?.rows ?? [];
    }
};

/**
 * Active milestone rewards (the tenure kind, e.g. subscriber-month badges gated
 * on total_months). Same world-readable rewards table, cached briefly. Claiming
 * is gated server-side by claim_reward against the real stored value, so a new
 * milestone badge is pure config (a cosmetics row + a rewards row) with no
 * client release.
 */
let milestoneRewardsCache: { at: number; rows: EventReward[] } | null = null;
export const listActiveMilestoneRewards = async (): Promise<EventReward[]> => {
    if (!supabase) return [];
    const now = Date.now();
    if (milestoneRewardsCache && now - milestoneRewardsCache.at < 5 * 60_000) return milestoneRewardsCache.rows;
    try {
        const { data, error } = await supabase
            .from('rewards')
            .select('id, title, reward_kind, reward_id')
            .eq('active', true)
            .eq('gate_kind', 'milestone');
        if (error) {
            Logger.warn('[Supabase] listActiveMilestoneRewards failed:', error.message);
            return milestoneRewardsCache?.rows ?? [];
        }
        const rows = (data ?? []) as EventReward[];
        milestoneRewardsCache = { at: now, rows };
        return rows;
    } catch (error) {
        Logger.warn('[Supabase] listActiveMilestoneRewards exception:', error);
        return milestoneRewardsCache?.rows ?? [];
    }
};

/**
 * Sync the caller's owned subscriber atmospheres. The server function
 * (grant_atmosphere_ownership, security definer) grants ownership of every
 * current subscriber atmosphere ONLY if the user is an active subscriber, so a
 * lapsed member keeps what they already own but stops accruing newly-added ones.
 * Fire-and-forget at login; idempotent. Silent when the RPC is not yet deployed.
 */
export const grantAtmosphereOwnership = async (userId: string): Promise<void> => {
    if (!supabase || !userId) return;
    try {
        const { error } = await supabase.rpc('grant_atmosphere_ownership', { p_twitch_user_id: userId });
        if (error && error.code !== '42883' && error.code !== 'PGRST202') {
            Logger.warn('[Supabase] grant_atmosphere_ownership failed:', error.message, error.code);
        }
    } catch (error) {
        Logger.warn('[Supabase] Exception in grant_atmosphere_ownership:', error);
    }
};

/**
 * Claim today's login accolades (seasonal windows + cake day) server-side. The
 * claim_login_accolades RPC enforces the window against the server clock and
 * reads the verified Twitch creation date, then grants idempotently. Returns the
 * ids freshly granted. Fire-and-forget on login / own-profile open.
 */
export const claimLoginAccolades = async (userId: string): Promise<string[]> => {
    if (!supabase || !userId) return [];
    try {
        const { data, error } = await supabase.rpc('claim_login_accolades', {
            p_twitch_user_id: userId,
        });
        if (error) {
            Logger.warn('[Supabase] claim_login_accolades failed:', error.message);
            return [];
        }
        const granted = (data as { granted?: string[] } | null)?.granted;
        return Array.isArray(granted) ? granted : [];
    } catch (error) {
        Logger.warn('[Supabase] claim_login_accolades exception:', error);
        return [];
    }
};

/**
 * Read a member's active loadout (slot -> equipped cosmetic slug) from the new
 * user_cosmetic_equipment table. World-readable, so it resolves any member's
 * equipped slots for rendering. Returns {} until the Stage 3 migration is applied
 * (the table + sync triggers populate it from the existing badge/theme writes).
 */
export const getActiveEquipment = async (userId: string): Promise<ActiveEquipment> => {
    if (!supabase || !userId) return {};
    try {
        const { data, error } = await supabase
            .from('user_cosmetic_equipment')
            .select('slot, cosmetic_slug')
            .eq('twitch_user_id', userId);
        if (error) {
            Logger.warn('[Supabase] equipment read failed:', error.message);
            return {};
        }
        const out: ActiveEquipment = {};
        for (const row of (data ?? []) as { slot: string; cosmetic_slug: string | null }[]) {
            out[row.slot as CosmeticSlot] = row.cosmetic_slug;
        }
        return out;
    } catch (error) {
        Logger.warn('[Supabase] equipment read exception:', error);
        return {};
    }
};

// ---------------------------------------------------------------------------
// Emote usage (most-used emotes from the member's own sent messages)
// ---------------------------------------------------------------------------

export interface EmoteUsage {
    emote_id: string;
    emote_name: string;
    provider: string;
    image_url: string | null;
    count: number;
}

/**
 * Increment usage of an emote for a user. Atomic via the `increment_emote_usage`
 * RPC, with a manual-upsert fallback. Fire-and-forget; never throws.
 */
export const incrementEmoteUsage = async (
    userId: string,
    emote: { id: string; name: string; provider: string; url: string },
    amount: number = 1,
): Promise<void> => {
    if (!supabase || !userId || !emote?.id) return;
    try {
        const { error } = await supabase.rpc('increment_emote_usage', {
            p_user_id: userId,
            p_emote_id: emote.id,
            p_emote_name: emote.name,
            p_provider: emote.provider,
            p_image_url: emote.url,
            p_amount: amount,
        });
        if (!error) return;
        await manualIncrementEmote(userId, emote, amount);
    } catch (error) {
        Logger.warn('[Supabase] incrementEmoteUsage failed:', error);
    }
};

const manualIncrementEmote = async (
    userId: string,
    emote: { id: string; name: string; provider: string; url: string },
    amount: number,
): Promise<void> => {
    if (!supabase) return;
    try {
        const { data: existing } = await supabase
            .from('user_emote_usage')
            .select('count')
            .eq('twitch_user_id', userId)
            .eq('emote_id', emote.id)
            .maybeSingle();
        const newCount = ((existing as { count?: number } | null)?.count || 0) + amount;
        await supabase.from('user_emote_usage').upsert(
            {
                twitch_user_id: userId,
                emote_id: emote.id,
                emote_name: emote.name,
                provider: emote.provider,
                image_url: emote.url,
                count: newCount,
                updated_at: new Date().toISOString(),
            },
            { onConflict: 'twitch_user_id,emote_id' },
        );
    } catch (error) {
        Logger.warn('[Supabase] manual emote increment failed:', error);
    }
};

/**
 * Get a user's most-used emotes, highest count first. Returns [] when Supabase
 * is unconfigured or the table is missing.
 */
export interface EmoteUsageSummary {
    top: EmoteUsage[];
    uniqueCount: number;
    totalCount: number;
}

/**
 * Get a user's emote usage: the top emotes (highest count first), the count of
 * distinct emotes used, and total uses. Returns zeros when Supabase is
 * unconfigured or the table is missing.
 */
export const getEmoteUsageSummary = async (
    userId: string,
    topLimit: number = 12,
): Promise<EmoteUsageSummary> => {
    const empty: EmoteUsageSummary = { top: [], uniqueCount: 0, totalCount: 0 };
    if (!supabase || !userId) return empty;
    try {
        const { data, error } = await supabase
            .from('user_emote_usage')
            .select('emote_id, emote_name, provider, image_url, count')
            .eq('twitch_user_id', userId)
            .order('count', { ascending: false })
            .limit(1000);
        if (error) {
            Logger.error('[Supabase] Failed to get emote usage:', error.message);
            return empty;
        }
        const rows = (data as EmoteUsage[]) || [];
        return {
            top: rows.slice(0, topLimit),
            uniqueCount: rows.length,
            totalCount: rows.reduce((s, e) => s + e.count, 0),
        };
    } catch (error) {
        Logger.error('[Supabase] Failed to get emote usage:', error);
        return empty;
    }
};

// ---------------------------------------------------------------------------
// Per-channel watch time (drives the "favorite channel" stat)
// ---------------------------------------------------------------------------

export interface ChannelWatch {
    channel_id: string;
    channel_login: string;
    channel_name: string;
    minutes: number;
}

/** The channel a user has spent the most time watching, or null. */
export const getFavoriteChannel = async (userId: string): Promise<ChannelWatch | null> => {
    if (!supabase || !userId) return null;
    try {
        const { data, error } = await supabase
            .from('user_channel_watch')
            .select('channel_id, channel_login, channel_name, minutes')
            .eq('twitch_user_id', userId)
            .order('minutes', { ascending: false })
            .limit(1)
            .maybeSingle();
        if (error) {
            Logger.error('[Supabase] Failed to get favorite channel:', error.message);
            return null;
        }
        return (data as ChannelWatch) || null;
    } catch (error) {
        Logger.error('[Supabase] Failed to get favorite channel:', error);
        return null;
    }
};

// ---------------------------------------------------------------------------
// Profile preferences (e.g. use your 7TV paint as your profile theme)
// ---------------------------------------------------------------------------

export interface ProfilePrefs {
    // True when the chosen theme is the 7TV paint (derived from profileTheme;
    // kept for back-compat with existing callers).
    paintTheme: boolean;
    // The theme source: 'tier' (free default) | 'paint' (7TV paint) | an
    // Atmosphere id like 'void'. Source of truth for the profile background.
    profileTheme: string;
    // Section keys the member has HIDDEN from their public profile (what other
    // members see). Empty = all visible. Keys: roast, twitch, lifetime, emotes,
    // accolades.
    hiddenSections: string[];
    // Whether the member's atmosphere (or Cologne look) also paints behind their
    // chat messages. False keeps it on the profile and hover card only.
    chatAtmosphere: boolean;
}

const defaultProfilePrefs = (): ProfilePrefs => ({
    paintTheme: false,
    profileTheme: 'tier',
    hiddenSections: [],
    chatAtmosphere: true,
});

export const getProfilePrefs = async (userId: string): Promise<ProfilePrefs> => {
    if (!userId) return defaultProfilePrefs();
    // Read in Rust (services/member_theme.rs), with the same defaults and the
    // legacy paint_theme derivation; a failed read answers the defaults.
    try {
        return await invoke<ProfilePrefs>('get_profile_prefs', { userId });
    } catch (error) {
        Logger.error('[Supabase] Failed to get profile prefs:', error);
        return defaultProfilePrefs();
    }
};

/**
 * Send a privileged write through the StreamNook API instead of Supabase.
 *
 * Returns false when the caller should fall back to the legacy direct write:
 * either the switch is off, or the request never reached the server. A server
 * REFUSAL (4xx) returns true, because the server deciding "no" is a real answer
 * and must not be retried against a table that would happily accept it.
 *
 * Identity is not passed. The server derives the acting user from the bearer
 * token that `streamnook_api_post` attaches in Rust, which is the entire point:
 * these tables are world-writable by user id under the anon key, so a body-
 * supplied id is exactly the hole being closed.
 */
interface ApiWriteResult {
    /** False means the legacy direct-Supabase path should run instead. */
    handled: boolean;
    /** Only meaningful when handled: whether the server accepted the write. */
    ok: boolean;
    error?: string;
}

/**
 * Post to the StreamNook API, unconditionally.
 *
 * `writeViaApi` below adds the server-controlled kill switch on top of this. A
 * write that MIGRATED from direct-Supabase wants that switch, because it has a
 * legacy path to fall back to. A write with no legacy equivalent must not be
 * gated on it: the flag defaults to false and the manifest does not currently
 * carry it, so gating a brand-new endpoint behind it means the endpoint is never
 * called at all.
 */
const postToApi = async (path: string, body: Record<string, unknown>): Promise<ApiWriteResult> => {
    try {
        const res = await invoke<{ status: number; ok: boolean; body: string }>(
            'streamnook_api_post',
            { path, body },
        );
        if (!res.ok) {
            Logger.warn(`[StreamNookAPI] ${path} -> ${res.status} ${res.body}`);
            // Surface it through the existing health channel so a refusal is
            // visible rather than silently dropping the user's change. 403 is the
            // server declining on entitlement, which is the same user-facing
            // shape as an RLS denial.
            reportWriteIssue({
                kind: res.status === 403 ? 'rls_denied' : 'other',
                lastSeen: new Date().toISOString(),
                detail: `${path} ${res.status}: ${res.body.slice(0, 200)}`,
            });
        }
        return { handled: true, ok: res.ok, error: res.ok ? undefined : res.body.slice(0, 200) };
    } catch (e) {
        // Transport failure only (no token, offline, command denied). The legacy
        // path is still open until the anon policies are revoked, so use it.
        Logger.warn(`[StreamNookAPI] ${path} unavailable, falling back:`, e);
        return { handled: false, ok: false };
    }
};

/**
 * The same post, behind the server-controlled kill switch.
 *
 * For writes that used to go straight to Supabase and can still do so: if the
 * new endpoint misbehaves, one field in the update manifest sends every build
 * back to the legacy path without a release. Fails OPEN to the legacy path, so a
 * Cloudflare blip cannot take writes down with it.
 */
const writeViaApi = async (path: string, body: Record<string, unknown>): Promise<ApiWriteResult> => {
    const { write_via_api: enabled } = await getClientConfig();
    if (!enabled) return { handled: false, ok: false };
    return postToApi(path, body);
};

/**
 * Set the member's profile theme ('tier' | 'paint' | an atmosphere id). Rust
 * owns the write (services/member_theme.rs: the server path when switched on,
 * else the direct upsert) and repaints the member's chat rows at once in every
 * window through `member-theme-changed`. Rejects when the write did not land.
 */
export const setProfileTheme = async (userId: string, theme: string): Promise<void> => {
    if (!userId) return;
    await invoke('set_profile_theme', { userId, theme });
};

/** Returns whether the change was saved, so the switch can undo itself. */
export const setChatAtmosphere = async (userId: string, show: boolean): Promise<boolean> => {
    if (!userId) return false;
    return invoke<boolean>('set_chat_atmosphere', { userId, show }).catch(() => false);
};

export const setHiddenSections = async (userId: string, sections: string[]): Promise<void> => {
    if (!userId) return;
    // Through the API first: the server writes the verified caller's row and
    // refreshes their public page at streamnook.app/u/<login>.
    if ((await postToApi('/api/v1/profile/prefs', { hidden_sections: sections })).handled) return;
    if (!supabase) return;
    try {
        const { error } = await supabase.from('user_profile_prefs').upsert(
            { twitch_user_id: userId, hidden_sections: sections, updated_at: new Date().toISOString() },
            { onConflict: 'twitch_user_id' },
        );
        if (error) Logger.error('[Supabase] Failed to set hidden sections:', error.message);
    } catch (error) {
        Logger.warn('[Supabase] setHiddenSections failed:', error);
    }
};

/**
 * Whether a member has an active StreamNook subscription, read from the
 * `stripe_subscriptions` table (the Stripe-backed support flow writes it). A
 * subscription counts when its latest row's status is 'active' or 'past_due' (the
 * payment-retry grace window). Requires the table to be SELECT-readable by the
 * anon key (the same way the cosmetics registry is).
 */
export const isStripeSubscriber = async (userId: string): Promise<boolean> => {
    if (!userId) return false;
    // Rust reads the latest row (active, past_due or trialing counts).
    return invoke<boolean>('is_stripe_subscriber', { userId }).catch(() => false);
};

/**
 * The member's own StreamNook membership facts for the profile Overview card:
 * current Stripe subscription state (status, renewal/end date, cancellation)
 * plus lifetime tenure (member since, total months). Both tables are
 * SELECT-readable under the anon key. Gifted (complimentary) memberships are
 * admin-only server-side, so a gifted member shows tenure without a billing
 * status here.
 */
export interface StreamNookMembership {
    /** Stripe status of the latest subscription, or null when none exists. */
    status: string | null;
    currentPeriodEnd: string | null;
    cancelAtPeriodEnd: boolean;
    /** When the current subscription started. */
    startedAt: string | null;
    /** When cancellation was requested (set while the sub runs out its paid period). */
    canceledAt: string | null;
    /** When a canceled subscription actually ended. */
    endedAt: string | null;
    totalMonths: number;
    /** First-ever paid month, the lifetime "member since" anchor. */
    firstSubscribedAt: string | null;
    lastPaidAt: string | null;
}

export const getStreamNookMembership = async (
    userId: string,
): Promise<StreamNookMembership | null> => {
    if (!supabase || !userId) return null;
    try {
        // select * so the read tolerates date columns that predate the
        // subscription_dates migration instead of erroring.
        const [subRes, tenureRes] = await Promise.all([
            supabase
                .from('stripe_subscriptions')
                .select('*')
                .eq('twitch_user_id', userId)
                .order('updated_at', { ascending: false })
                .limit(1)
                .maybeSingle(),
            supabase
                .from('user_subscriber_state')
                .select('*')
                .eq('twitch_user_id', userId)
                .maybeSingle(),
        ]);
        if (subRes.error) Logger.warn('[Supabase] membership sub read failed:', subRes.error.message);
        if (tenureRes.error) Logger.warn('[Supabase] membership tenure read failed:', tenureRes.error.message);

        const sub = subRes.data as Record<string, unknown> | null;
        const tenure = tenureRes.data as Record<string, unknown> | null;
        if (!sub && !tenure) return null;

        return {
            status: (sub?.status as string | undefined) ?? null,
            currentPeriodEnd: (sub?.current_period_end as string | undefined) ?? null,
            cancelAtPeriodEnd: Boolean(sub?.cancel_at_period_end),
            startedAt: (sub?.started_at as string | undefined) ?? (sub?.created_at as string | undefined) ?? null,
            canceledAt: (sub?.canceled_at as string | undefined) ?? null,
            endedAt: (sub?.ended_at as string | undefined) ?? null,
            totalMonths: Number(tenure?.total_months ?? 0),
            firstSubscribedAt: (tenure?.first_subscribed_at as string | undefined) ?? null,
            lastPaidAt: (tenure?.last_paid_at as string | undefined) ?? null,
        };
    } catch (error) {
        Logger.warn('[Supabase] getStreamNookMembership failed:', error);
        return null;
    }
};

// ============================================================================
// StreamNook registries: members, cosmetics, atmospheres (held in Rust)
// ============================================================================
// Rust reads the member registry (`user_numbers`), the cosmetics catalog with
// every member's grants and worn badge, and the atmosphere catalog once for
// every window (services/cosmetics_registry.rs), keeps them fresh from its own
// realtime socket, on focus and on resume, and keeps the last good copy when a
// read fails. This is the page's copy of that answer, for the synchronous reads
// render paths need; `sn-registry-changed` says when to read it again.

interface RegistrySnapshot {
    version: number;
    membersLoaded: boolean;
    cosmeticsLoaded: boolean;
    atmospheresLoaded: boolean;
    members: Record<string, number>;
    catalog: CosmeticCatalogEntry[];
    grants: Record<string, string[]>;
    /** The badge each member wears, already checked against what they own. */
    worn: Record<string, string>;
    atmospheres: Atmosphere[];
}

export interface CosmeticCatalogEntry {
    slug: string;
    name: string;
    description: string | null;
    kind: string;
    asset_path: string;
    animated: boolean;
    payment_type: string | null;
    ko_fi_url: string | null;
    stripe_url: string | null;
    sort_order: number;
    is_active: boolean;
    is_default: boolean;
    // Owner-only / staged cosmetics: kept out of the public "all badges"
    // collection. Still ownable and equippable by whoever has been granted it.
    hidden?: boolean;
    // Set on Contributor badge tiers: the shipped contribution points that earn
    // the tier. These are granted by the database, never bought.
    contribution_min_points?: number | null;
}

let snRegistry: Map<string, number> = new Map();
let cosmeticsCatalog: Map<string, CosmeticCatalogEntry> = new Map();
let cosmeticsEntitlements: Map<string, Set<string>> = new Map(); // twitch_user_id -> granted slugs
let cosmeticsWorn: Map<string, string> = new Map();              // twitch_user_id -> worn badge slug
let atmospheresCatalog: Map<string, Atmosphere> = new Map();
let snRegistryLoaded = false;
let cosmeticsLoaded = false;
let atmospheresLoaded = false;
let snRegistryVersion = 0;
let cosmeticsVersion = 0;
let atmospheresVersion = 0;
const snRegistryVersionSubscribers = new Set<() => void>();
const cosmeticsVersionSubscribers = new Set<() => void>();
const atmospheresVersionSubscribers = new Set<() => void>();
let snapshotLoading: Promise<void> | null = null;
let snapshotReloadQueued = false;
let snapshotListening = false;

const notify = (subscribers: Set<() => void>, label: string) => {
    for (const cb of subscribers) {
        try { cb(); } catch (e) { Logger.error(`[Supabase] ${label} subscriber error:`, e); }
    }
};

const bumpStreamNookRegistryVersion = () => {
    snRegistryVersion++;
    notify(snRegistryVersionSubscribers, 'Registry version');
};

const bumpCosmeticsVersion = () => {
    cosmeticsVersion++;
    notify(cosmeticsVersionSubscribers, 'Cosmetics');
};

const bumpAtmospheresVersion = () => {
    atmospheresVersion++;
    notify(atmospheresVersionSubscribers, 'Atmospheres');
};

// Each atmosphere as Rust's snapshot delivered it, before its art on
// cdn.streamnook.app is pointed at Rust's disk copies. The catalog is rebuilt
// from these when a copy lands, so art the webview could not load still arrives.
let atmosphereSources: Map<string, Atmosphere> = new Map();
// Atmospheres whose art was asked of Rust since the catalog loaded: `id` for the
// look alone, `id+all` for the look plus the Cologne add-ons (coin, frame).
const atmosphereArtAsked = new Set<string>();

/** An atmosphere with its CDN art pointed at Rust's disk copies, where there are some. */
const withAtmosphereArtCopies = (a: Atmosphere): Atmosphere => ({
    ...a,
    swatch: withStreamnookArtCopies(a.swatch),
    baseLayers: withStreamnookArtCopies(a.baseLayers),
    image: withStreamnookArtCopies(a.image),
    layers: withStreamnookArtCopies(a.layers),
    layers2: withStreamnookArtCopies(a.layers2),
    chromeTexture: withStreamnookArtCopies(a.chromeTexture),
    chromeCoin: withStreamnookArtCopies(a.chromeCoin),
    chromeFrame: withStreamnookArtCopies(a.chromeFrame),
});

const sameAtmosphereArt = (a: Atmosphere, b: Atmosphere): boolean =>
    a.swatch === b.swatch && a.baseLayers === b.baseLayers && a.image === b.image &&
    a.layers === b.layers && a.layers2 === b.layers2 && a.chromeTexture === b.chromeTexture &&
    a.chromeCoin === b.chromeCoin && a.chromeFrame === b.chromeFrame;

/**
 * Ask Rust for copies of an atmosphere's art the first time something reads it,
 * so art stays lazy: only atmospheres a picker lists or a member wears are
 * fetched. A listing needs the look; a worn Cologne look can add its coin and
 * frame, so a single read asks for those too.
 */
const askAtmosphereArt = (id: string, withAddOns: boolean): void => {
    const key = withAddOns ? `${id}+all` : id;
    if (atmosphereArtAsked.has(key)) return;
    atmosphereArtAsked.add(key);
    const a = atmosphereSources.get(id);
    if (!a) return;
    const values = [a.swatch, a.baseLayers, a.image, a.layers, a.layers2, a.chromeTexture];
    if (withAddOns) values.push(a.chromeCoin, a.chromeFrame);
    for (const url of values.flatMap(streamnookArtUrlsIn)) requestStreamnookArt(url);
};

// A copy landed (or the cache was cleared): repoint the catalog and repaint the
// atmospheres whose art changed. Unchanged entries keep their identity.
subscribeStreamnookArt(() => {
    if (atmosphereSources.size === 0) return;
    let changed = false;
    const next = new Map<string, Atmosphere>();
    for (const [id, source] of atmosphereSources) {
        const mapped = withAtmosphereArtCopies(source);
        const current = atmospheresCatalog.get(id);
        if (current && sameAtmosphereArt(current, mapped)) {
            next.set(id, current);
        } else {
            next.set(id, mapped);
            changed = true;
        }
    }
    if (changed) {
        atmospheresCatalog = next;
        bumpAtmospheresVersion();
    }
});


/**
 * Tell everything that reads "is this person a member" to look again.
 *
 * Exported for the cross-platform alias resolver, which answers the same
 * question for a Kick or YouTube chatter and lands asynchronously. Chat resolves
 * each chatter ONCE, so a member who spoke before their claim was known would be
 * classified as a non-member for the whole session; the re-resolve bridge in
 * `chatUserStore` already listens to this counter, so bumping it is the entire
 * fix rather than a second mechanism.
 */
export const notifyMemberIdentityChanged = (): void => {
    bumpStreamNookRegistryVersion();
};

/** Takes Rust's answer into the maps the synchronous reads use. */
const applySnapshot = (snap: RegistrySnapshot) => {
    snRegistry = new Map(Object.entries(snap.members ?? {}));
    const nextCatalog = new Map<string, CosmeticCatalogEntry>();
    for (const row of snap.catalog ?? []) nextCatalog.set(row.slug, row);
    // Dev builds get candidate badge slots merged in after the live rows
    // (see devCosmetics.ts). Tree-shaken out of production.
    if (import.meta.env.DEV) {
        for (const c of DEV_COSMETICS) nextCatalog.set(c.slug, c);
    }
    cosmeticsCatalog = nextCatalog;
    cosmeticsEntitlements = new Map(Object.entries(snap.grants ?? {}).map(([id, slugs]) => [id, new Set(slugs)]));
    cosmeticsWorn = new Map(Object.entries(snap.worn ?? {}));
    // A dev badge equipped locally is restored over the server's answer, since
    // the server never heard about it. Dev builds only.
    const devActive = readDevActiveCosmetic();
    if (devActive) cosmeticsWorn.set(devActive.userId, devActive.slug);
    const nextAtmospheres = new Map<string, Atmosphere>();
    for (const a of snap.atmospheres ?? []) nextAtmospheres.set(a.id, a);
    // Dev builds get candidate slots merged in after the live rows, so an asset
    // can be judged in the real app before it ships (see devAtmospheres.ts).
    if (import.meta.env.DEV) {
        for (const a of DEV_ATMOSPHERES) nextAtmospheres.set(a.id, a);
    }
    // Atmosphere art loads lazily at its render sites, and its disk copies
    // follow the same rule: asked for on the first read (askAtmosphereArt).
    atmosphereSources = nextAtmospheres;
    atmosphereArtAsked.clear();
    atmospheresCatalog = new Map([...nextAtmospheres].map(([id, a]) => [id, withAtmosphereArtCopies(a)]));
    snRegistryLoaded = snap.membersLoaded;
    cosmeticsLoaded = snap.cosmeticsLoaded;
    atmospheresLoaded = snap.atmospheresLoaded;
    bumpStreamNookRegistryVersion();
    bumpCosmeticsVersion();
    bumpAtmospheresVersion();
};

/** Reads Rust's registries into this page. One read at a time; a change that
 *  lands mid-read queues exactly one more. */
const loadSnapshot = (): Promise<void> => {
    if (snapshotLoading) { snapshotReloadQueued = true; return snapshotLoading; }
    snapshotLoading = (async () => {
        try {
            applySnapshot(await invoke<RegistrySnapshot>('sn_registry_snapshot'));
        } catch (e) {
            Logger.warn('[Supabase] registry snapshot unavailable:', e);
        } finally {
            snapshotLoading = null;
            if (snapshotReloadQueued) { snapshotReloadQueued = false; void loadSnapshot(); }
        }
    })();
    return snapshotLoading;
};

/** Reads the snapshot once and follows Rust's change event from then on. */
const ensureRegistries = (): void => {
    if (snapshotListening) return;
    snapshotListening = true;
    void loadSnapshot();
    void import('@tauri-apps/api/event')
        .then(({ listen }) => listen('sn-registry-changed', () => { void loadSnapshot(); }))
        .catch((e) => Logger.warn('[Supabase] registry change listener unavailable:', e));
};

/**
 * Subscribe to the StreamNook user registry. The first call reads Rust's copy;
 * the callback runs with the member map on every change. Returns an unsubscribe.
 */
export const subscribeToStreamNookRegistry = (
    callback?: (map: Map<string, number>) => void
): (() => void) | null => {
    const versionCb = () => callback?.(snRegistry);
    snRegistryVersionSubscribers.add(versionCb);
    if (snRegistryLoaded) callback?.(snRegistry);
    ensureRegistries();
    return () => { snRegistryVersionSubscribers.delete(versionCb); };
};

/** Sync read: is this Twitch user in the registry? */
export const isStreamNookUser = (userId: string | undefined | null): boolean => {
    if (!userId) return false;
    return snRegistry.has(userId);
};

/** Sync read: get the user number, or null if not registered. */
export const getStreamNookUserNumber = (userId: string | undefined | null): number | null => {
    if (!userId) return null;
    return snRegistry.get(userId) ?? null;
};

/** For React useSyncExternalStore: subscribe to version bumps. */
export const subscribeStreamNookRegistryVersion = (cb: () => void): (() => void) => {
    snRegistryVersionSubscribers.add(cb);
    ensureRegistries();
    return () => { snRegistryVersionSubscribers.delete(cb); };
};

/** For React useSyncExternalStore: read the current version. */
export const getStreamNookRegistryVersion = (): number => snRegistryVersion;

/**
 * Subscribe to the cosmetics registry. The first call reads Rust's copy;
 * subsequent calls just register the listener.
 */
export const subscribeToCosmeticsRegistry = (
    callback?: () => void,
): (() => void) | null => {
    const cb = () => callback?.();
    cosmeticsVersionSubscribers.add(cb);
    if (cosmeticsLoaded) callback?.();
    ensureRegistries();
    return () => { cosmeticsVersionSubscribers.delete(cb); };
};

/** Sync read: catalog row by slug. */
export const getCosmeticBySlug = (slug: string | undefined | null): CosmeticCatalogEntry | null => {
    if (!slug) return null;
    return cosmeticsCatalog.get(slug) ?? null;
};

/** Sync read: full catalog, sorted by sort_order. */
export const getAllCosmetics = (): CosmeticCatalogEntry[] => {
    return Array.from(cosmeticsCatalog.values()).sort((a, b) => a.sort_order - b.sort_order);
};

/** Sync read: which slugs has this user been explicitly granted? Does NOT
 *  include `is_default` cosmetics that everyone can equip — use
 *  `getOwnedCosmeticSlugs` for the rendering / picker check. */
export const getUserCosmeticSlugs = (userId: string | undefined | null): Set<string> => {
    if (!userId) return new Set();
    return cosmeticsEntitlements.get(userId) ?? new Set();
};

/** Sync read: every slug this user can equip — explicit entitlements PLUS
 *  every `is_default` cosmetic (the rule Rust's `owned_slugs` applies). */
export const getOwnedCosmeticSlugs = (userId: string | undefined | null): Set<string> => {
    const owned = new Set<string>();
    for (const cosmetic of cosmeticsCatalog.values()) {
        // Dev-only candidate slots always count as owned on a dev build; that
        // is what they are for. isDevCosmetic is always false in production.
        if (cosmetic.is_default || isDevCosmetic(cosmetic.slug)) owned.add(cosmetic.slug);
    }
    if (userId) {
        const explicit = cosmeticsEntitlements.get(userId);
        if (explicit) for (const slug of explicit) owned.add(slug);
    }
    return owned;
};

/** Whether the cosmetics registry has actually loaded.
 *
 *  An empty registry and "this member owns nothing" are the same answer from
 *  `getOwnedCosmeticSlugs`, so callers must allow rather than deny when this is
 *  false. */
export const isCosmeticsRegistryLoaded = (): boolean => cosmeticsLoaded && cosmeticsCatalog.size > 0;

/** Sync read: which slug is this user currently displaying? Rust already
 *  dropped a worn slug the member no longer owns or that is not a badge. */
export const getActiveCosmeticSlug = (userId: string | undefined | null): string | null => {
    if (!userId) return null;
    return cosmeticsWorn.get(userId) ?? null;
};

/** For React useSyncExternalStore: subscribe to version bumps. */
export const subscribeCosmeticsVersion = (cb: () => void): (() => void) => {
    cosmeticsVersionSubscribers.add(cb);
    ensureRegistries();
    return () => { cosmeticsVersionSubscribers.delete(cb); };
};

/** For React useSyncExternalStore: read the current version. */
export const getCosmeticsVersion = (): number => cosmeticsVersion;

/**
 * Subscribe to the atmospheres registry. Serves the copy held now at once, and
 * asks Rust to re-read (a newly earned atmosphere appears without a relaunch).
 */
export const subscribeToAtmospheresRegistry = (
    callback?: () => void,
): (() => void) | null => {
    const cb = () => callback?.();
    atmospheresVersionSubscribers.add(cb);
    if (atmospheresLoaded) callback?.();
    ensureRegistries();
    void invoke('sn_refresh_registries').catch(() => {});
    return () => { atmospheresVersionSubscribers.delete(cb); };
};

// ─── Live profile-theme changes (cross-user) ────────────────────────────────
// Rust follows `user_profile_prefs` on its realtime socket and resolves each
// member's theme (services/member_theme.rs); `member-theme-changed` carries the
// result. `chatAtmosphere` is false when the member keeps their theme off chat.
type ProfileThemeSubscriber = (userId: string, profileTheme: string | null, chatAtmosphere?: boolean) => void;
const profileThemeSubscribers = new Set<ProfileThemeSubscriber>();
let profileThemeListening = false;

export interface MemberTheme {
    memberId: string;
    theme?: string | null;
    atmosphereId?: string | null;
    cologne?: { coin: boolean; frame: boolean } | null;
    hiddenInChat: boolean;
}

export const subscribeToProfileThemeChanges = (
    callback: ProfileThemeSubscriber,
): (() => void) | null => {
    profileThemeSubscribers.add(callback);
    if (!profileThemeListening) {
        profileThemeListening = true;
        void import('@tauri-apps/api/event')
            .then(({ listen }) => listen<MemberTheme>('member-theme-changed', (event) => {
                const t = event.payload;
                if (!t?.memberId) return;
                for (const fn of profileThemeSubscribers) {
                    try { fn(t.memberId, t.theme ?? null, !t.hiddenInChat); }
                    catch (e) { Logger.error('[Supabase] profile theme subscriber error:', e); }
                }
            }))
            .catch((e) => Logger.warn('[Supabase] theme change listener unavailable:', e));
    }
    return () => { profileThemeSubscribers.delete(callback); };
};

/** Sync read: an atmosphere definition by id (null if unknown / not loaded). */
export const getAtmosphereEntry = (id: string | undefined | null): Atmosphere | null => {
    if (!id) return null;
    const atmosphere = atmospheresCatalog.get(id) ?? null;
    if (atmosphere) askAtmosphereArt(id, true);
    return atmosphere;
};

/** Sync read: the full catalog (already in sort_order from the load query). */
export const listAtmosphereEntries = (): Atmosphere[] => {
    for (const id of atmospheresCatalog.keys()) askAtmosphereArt(id, false);
    return Array.from(atmospheresCatalog.values());
};

/** For React useSyncExternalStore: subscribe to / read the catalog version. */
export const subscribeAtmospheresVersion = (cb: () => void): (() => void) => {
    atmospheresVersionSubscribers.add(cb);
    ensureRegistries();
    return () => { atmospheresVersionSubscribers.delete(cb); };
};
export const getAtmospheresVersion = (): number => atmospheresVersion;

/** Resolves once the catalog has loaded (or Rust could not answer), so async
 *  resolvers never read an empty catalog. */
export const whenAtmospheresReady = (): Promise<void> => {
    if (atmospheresLoaded) return Promise.resolve();
    ensureRegistries();
    return snapshotLoading ?? loadSnapshot();
};

/**
 * Re-read the entitlement-bearing registries (cosmetics, members, atmospheres)
 * in Rust. Wired to window focus, so a purchase completed on streamnook.app
 * while realtime happened to be reconnecting still shows with no restart.
 */
export const refreshEntitlementRegistries = (): void => {
    void invoke('sn_refresh_registries').catch(() => {});
};

// ─── Profile snapshot cache (stale-while-revalidate) ───────────────────────
// A per-user CACHE of a member's fully-resolved profile so the public profile
// overlay can paint instantly from ONE read, then revalidate the live sources
// in the background. NOT a source of truth (7TV/Twitch stay authoritative); a
// stale row self-corrects on the next open. See migration 20260605000001.

// Single JSONB blob; `v` lets the shape evolve without a migration. Cosmetic
// sub-objects are kept loose (resolved upstream) to avoid coupling to deep 7TV
// types — consumers treat them as opaque render data.
export interface ProfileSnapshot {
    v: 1;
    identity: { login: string; displayName: string; avatar: string };
    profileTheme: string; // 'tier' | 'paint' | <atmosphere id>
    hiddenSections: string[];
    memberNumber: number | null;
    cosmeticSlug: string | null; // active StreamNook cosmetic
    namePaint: Record<string, unknown> | null; // resolved 7TV paint style (CSSProperties)
    seventvBadge: Record<string, unknown> | null; // active 7TV badge
    wornBadges: {
        twitch: { src: string; title: string } | null;
        thirdParty: Array<{ key?: string; provider?: string; title?: string; src: string }>;
        bttvPro: { src: string; title: string } | null;
    };
    counts: { paints: number; badges: number; sn: number };
    stats: { messages: number; streams: number; hours: number } | null;
    accolades: string[]; // earned accolade ids
    favoriteChannel: unknown | null;
    ivr: { followers: number | null; createdAt: string | null; roles: unknown } | null;
}

/** Read a member's cached profile snapshot (null if none / unknown version). */
export const getProfileSnapshot = async (
    userId: string,
): Promise<{ snapshot: ProfileSnapshot; updatedAt: string } | null> => {
    if (!supabase || !userId) return null;
    try {
        const { data, error } = await supabase
            .from('user_profile_snapshot')
            .select('snapshot, updated_at')
            .eq('twitch_user_id', userId)
            .maybeSingle();
        if (error || !data) return null;
        const row = data as { snapshot: ProfileSnapshot | null; updated_at: string };
        if (!row.snapshot || row.snapshot.v !== 1) return null;
        return { snapshot: row.snapshot, updatedAt: row.updated_at };
    } catch (e) {
        Logger.warn('[Supabase] getProfileSnapshot failed:', e);
        return null;
    }
};

/** Write/refresh a member's cached profile snapshot (cache-aside fill). */
export const upsertProfileSnapshot = async (
    userId: string,
    snapshot: ProfileSnapshot,
): Promise<void> => {
    if (!supabase || !userId) return;
    try {
        const { error } = await supabase.from('user_profile_snapshot').upsert(
            { twitch_user_id: userId, snapshot, updated_at: new Date().toISOString() },
            { onConflict: 'twitch_user_id' },
        );
        if (error) Logger.warn('[Supabase] upsertProfileSnapshot failed:', error.message);
    } catch (e) {
        Logger.warn('[Supabase] upsertProfileSnapshot exception:', e);
    }
};

/**
 * Optimistically patch ONLY the profileTheme of a member's cached snapshot after
 * they change their atmosphere, so the next profile-card open (theirs or anyone
 * else's) reflects it without waiting for the lazy stale-rewrite, and so the
 * instant snapshot render can't clobber the fresh prefs with the old theme. The
 * other snapshot fields are untouched (a theme change doesn't affect them), so it
 * stays consistent. No-op when no snapshot exists yet (the overlay builds fresh).
 */
export const patchProfileSnapshotTheme = async (userId: string, theme: string): Promise<void> => {
    if (!supabase || !userId) return;
    try {
        const existing = await getProfileSnapshot(userId);
        if (!existing || existing.snapshot.profileTheme === theme) return;
        await upsertProfileSnapshot(userId, { ...existing.snapshot, profileTheme: theme });
    } catch (e) {
        Logger.warn('[Supabase] patchProfileSnapshotTheme failed:', e);
    }
};

/** Read a member's Twitch identity (login/display/avatar) from OUR `users` table,
 *  so the profile overlay can paint instantly without a Twitch Helix round-trip.
 *  Helix becomes a background revalidation. Null for users we've never recorded
 *  (e.g. non-members), which fall back to Helix. */
export const getUserIdentity = async (
    userId: string,
): Promise<{ login: string; displayName: string; avatar: string } | null> => {
    if (!supabase || !userId) return null;
    try {
        const { data, error } = await supabase
            .from('users')
            .select('username, display_name, avatar_url')
            .eq('id', userId)
            .maybeSingle();
        if (error || !data) return null;
        const row = data as { username?: string; display_name?: string; avatar_url?: string };
        if (!row.username) return null;
        return {
            login: row.username,
            displayName: row.display_name || row.username,
            avatar: row.avatar_url || '',
        };
    } catch (e) {
        Logger.warn('[Supabase] getUserIdentity failed:', e);
        return null;
    }
};

/**
 * Set the worn StreamNook badge for a user. Pass null to revert to the default
 * (the plain StreamNook mark). The page's copy flips at once so the picker
 * swaps instantly; Rust writes it (services/cosmetics_registry.rs: the server
 * path when switched on, else the direct upsert), flips its own registry, and
 * rolls both back through `sn-registry-changed` if the write is refused.
 */
export const setActiveCosmetic = async (
    userId: string,
    slug: string | null,
): Promise<{ ok: boolean; error?: string }> => {
    if (!userId) return { ok: false, error: 'no userId' };

    const prev = cosmeticsWorn.get(userId) ?? null;
    if (slug) cosmeticsWorn.set(userId, slug);
    else cosmeticsWorn.delete(userId);
    bumpCosmeticsVersion();

    // Dev-only candidate badges equip LOCALLY: the server would refuse a slug
    // it has no row for. Persisted so a reload keeps it; cleared the moment a
    // real badge is chosen, so the server path below takes over again.
    if (isDevCosmetic(slug)) {
        writeDevActiveCosmetic(userId, slug);
        return { ok: true };
    }
    if (isDevCosmetic(prev)) writeDevActiveCosmetic(userId, null);

    try {
        await invoke('equip_streamnook_badge', { userId, slug });
        return { ok: true };
    } catch (e: unknown) {
        if (prev) cosmeticsWorn.set(userId, prev);
        else cosmeticsWorn.delete(userId);
        bumpCosmeticsVersion();
        const msg = typeof e === 'string' ? e : e instanceof Error ? e.message : 'Could not equip that badge.';
        return { ok: false, error: msg };
    }
};

if (import.meta.env.DEV && typeof window !== 'undefined') {
    (window as any).__snDebug = {
        registry: () => snRegistry,
        size: () => snRegistry.size,
        loaded: () => snRegistryLoaded,
        version: () => snRegistryVersion,
        isMember: (id: string) => snRegistry.has(id),
        number: (id: string) => snRegistry.get(id) ?? null,
        reload: () => loadSnapshot(),
        cosmetics: {
            catalog: () => cosmeticsCatalog,
            entitlements: () => cosmeticsEntitlements,
            worn: () => cosmeticsWorn,
            loaded: () => cosmeticsLoaded,
            version: () => cosmeticsVersion,
            reload: () => loadSnapshot(),
        },
    };
}

// Export default object for convenience
export default {
    getSupabaseClient,
    isSupabaseConfigured,
    trackPresence,
    updatePresence,
    upsertUser,
    getOnlineCount,
    getOnlinePresenceSnapshot,
    subscribeToOnlineCount,
    subscribeToOnlinePresence,
    isPresenceReady,
    incrementStat,
    getLastWriteIssue,
    subscribeToWriteIssues,
    getUserStats,
    subscribeToStreamNookRegistry,
    isStreamNookUser,
    getStreamNookUserNumber,
    subscribeToCosmeticsRegistry,
    getCosmeticBySlug,
    getAllCosmetics,
    getUserCosmeticSlugs,
    getOwnedCosmeticSlugs,
    getActiveCosmeticSlug,
    subscribeCosmeticsVersion,
    getCosmeticsVersion,
    setActiveCosmetic,
    refreshEntitlementRegistries,
};
