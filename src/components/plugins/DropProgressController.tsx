import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, emit } from '@tauri-apps/api/event';
import { useAppStore } from '../../stores/AppStore';
import type {
    DropProgressStatus,
    DropChannel,
    CurrentDropInfo,
    DropCampaign,
    TimeBasedDrop,
} from '../../types';
import { Logger } from '../../utils/logger';
import { useWatchDrops } from '../../stores/watchDropsStore';
import { EMPTY_DROP_STATUS, statusFromView, tierUpdates } from '../../utils/watchDropsStatus';


// The single contract id an external drops provider (e.g. an opt-in plugin)
// registers via `plugins_provides` to take over the drop-progress display. The
// core knows nothing else about who provides it. Rename in coordination with
// the provider when its manifest is updated.
const EXTERNAL_DROPS_FEATURE = 'drops.automation';
// The generic status slot a provider pushes its live progress into.
const EXTERNAL_DROPS_STATUS_SLOT = 'drops.status';

// Shape an external provider pushes into its status slot. `active` = a session/
// target is set; `is_active` = actively progressing a channel right now. These
// are the provider's contract field names (kept until a coordinated rename).
interface ProviderStatusValue {
    active?: boolean;
    is_active?: boolean;
    game_name?: string | null;
    campaign_id?: string | null;
    channel_login?: string | null;
    drop_name?: string | null;
    current_minutes?: number | null;
    required_minutes?: number | null;
    // Every reward tier's minutes for the current campaign, so the Drops panel's
    // per-tier bars + "next: X in Ym" can update live off provider pushes (not
    // just the single current drop the title bar uses).
    drops?: Array<{
        drop_id?: string;
        name?: string;
        current_minutes?: number;
        required_minutes?: number;
        is_claimed?: boolean;
    }> | null;
}

/**
 * Drives the drop-progress display (title-bar badge + hover + Drops overlay)
 * from two interchangeable sources, with no knowledge of any specific plugin:
 *
 *  - NATIVE watch-to-earn: Rust decides which drop you are earning on the
 *    channel you watch and how far along it is (services/watch_drops.rs, one
 *    model shared with the chat bar and the phones). This only translates that
 *    model into the store fields and the 'drop-progress' /
 *    'drops-progress-update' events the UI already listens for.
 *  - EXTERNAL provider: when a plugin registers for the generic drops feature, it
 *    takes over: this translates its `drops.status` pushes into the same events,
 *    and the native source stands down. The plugin can progress drops without an
 *    on-screen stream; the core neither knows nor cares how.
 *
 * Either way the same display lights up identically, even while the Drops overlay
 * is closed. `externalDropsProvider` (published to the store) lets the rest of
 * the app render provider-only controls only when a provider is present.
 */
export default function DropProgressController() {
    const externalDropsProvider = useAppStore((s) => s.externalDropsProvider);

    // ---- External provider: detect it, and bridge its status pushes ----
    const providerRef = useRef<string | null>(null);
    const campaignsRef = useRef<DropCampaign[]>([]);
    const fetchingRef = useRef(false);
    const lastFetchRef = useRef(0);

    useEffect(() => {
        let disposed = false;
        const unlisteners: (() => void)[] = [];

        const refreshCampaigns = async () => {
            if (fetchingRef.current || Date.now() - lastFetchRef.current < 60_000) return;
            lastFetchRef.current = Date.now();
            fetchingRef.current = true;
            try {
                const list = await invoke<DropCampaign[]>('get_active_drop_campaigns');
                if (!disposed && Array.isArray(list)) campaignsRef.current = list;
            } catch {
                /* campaigns unavailable */
            } finally {
                fetchingRef.current = false;
            }
        };

        // Resolve a reward NAME from campaign data when a provider's push doesn't
        // carry one: the unclaimed, still-incomplete drop closest to finishing.
        const resolveDropName = (campaignId: string): string => {
            const campaign = campaignsRef.current.find((c) => c.id === campaignId);
            if (!campaign) return '';
            const remaining = (d: TimeBasedDrop) =>
                (d.progress?.required_minutes_watched ?? d.required_minutes_watched) -
                (d.progress?.current_minutes_watched ?? 0);
            let best: TimeBasedDrop | null = null;
            for (const d of campaign.time_based_drops) {
                if ((d.required_minutes_watched ?? 0) <= 0 || d.progress?.is_claimed) continue;
                if (remaining(d) <= 0) continue;
                if (!best || remaining(d) < remaining(best)) best = d;
            }
            const pick = best
                ?? campaign.time_based_drops.find((d) => (d.required_minutes_watched ?? 0) > 0)
                ?? null;
            return pick ? (pick.benefit_edges?.[0]?.name || pick.name || '') : '';
        };

        const refreshProvider = async () => {
            try {
                const id = await invoke<string | null>('plugins_provides', { feature: EXTERNAL_DROPS_FEATURE });
                if (disposed) return;
                const next = id ?? null;
                const had = providerRef.current;
                providerRef.current = next;
                // Publish provider presence so provider-only controls render only
                // when a provider is actually present.
                useAppStore.getState().setExternalDropsProvider(!!next);
                if (next) refreshCampaigns();
                // Provider went away (disabled/removed). It can't push a final
                // stopped status once its process is gone, so stand the display
                // down here, otherwise a reopened overlay seeds from stale status.
                if (had && !next) clearStatus();
            } catch {
                /* plugin host unavailable */
            }
        };

        const clearStatus = () => {
            const store = useAppStore.getState();
            store.setLiveDropProgress(null);
            store.setDropProgressActive(false);
            store.setDropProgressComplete(false);
            emit('drop-progress', {
                active: false,
                current_channel: null,
                current_campaign: null,
                current_drop: null,
                eligible_channels: [],
                last_update: new Date().toISOString(),
            } as DropProgressStatus).catch(() => {});
        };

        refreshProvider();

        const setup = async () => {
            const unState = await listen('plugin://state-changed', () => refreshProvider());
            const unStatus = await listen<{ slot: string; value: ProviderStatusValue }>(
                'plugin://status',
                (event) => {
                    if (!providerRef.current || event.payload.slot !== EXTERNAL_DROPS_STATUS_SLOT) return;
                    const v = event.payload.value || {};

                    const channel: DropChannel | null = v.channel_login
                        ? {
                              id: '',
                              name: v.channel_login,
                              display_name: v.channel_login,
                              game_name: v.game_name ?? '',
                              viewer_count: 0,
                              is_live: true,
                              drops_enabled: true,
                          }
                        : null;
                    let dropName = (v.drop_name ?? '').trim();
                    if (!dropName && v.campaign_id) {
                        dropName = resolveDropName(v.campaign_id);
                        if (!dropName) refreshCampaigns();
                    }
                    const drop: CurrentDropInfo | null = v.game_name
                        ? {
                              campaign_id: v.campaign_id ?? '',
                              campaign_name: '',
                              drop_id: v.campaign_id ?? '',
                              drop_name: dropName,
                              required_minutes: v.required_minutes ?? 0,
                              current_minutes: v.current_minutes ?? 0,
                              game_name: v.game_name,
                          }
                        : null;

                    const status: DropProgressStatus = {
                        active: !!v.is_active,
                        current_channel: channel,
                        current_campaign: v.campaign_id ?? null,
                        current_drop: drop,
                        eligible_channels: [],
                        last_update: new Date().toISOString(),
                    };
                    emit('drop-progress', status).catch(() => {});
                    // The title bar updates from 'drop-progress' above (the single
                    // current drop). The Drops panel's per-tier reward bars and the
                    // "next: X in Ym" countdown listen for 'drops-progress-update'
                    // instead — which the native watch path emits but this provider
                    // path did not, so they only refreshed on a full reload. The
                    // provider now sends every tier's minutes in `drops`; forward
                    // them as the same per-tier event so the panel updates live too.
                    if (Array.isArray(v.drops) && v.campaign_id) {
                        const now = Date.now();
                        for (const d of v.drops) {
                            const req = d?.required_minutes ?? 0;
                            if (!d?.drop_id || req <= 0) continue;
                            emit('drops-progress-update', {
                                drop_id: d.drop_id,
                                current_minutes: d.current_minutes ?? 0,
                                required_minutes: req,
                                timestamp: now,
                                campaign_id: v.campaign_id,
                            }).catch(() => {});
                        }
                    }
                    useAppStore.getState().setDropProgressActive(!!v.is_active);
                    useAppStore.getState().setLiveDropProgress(v.active ? status : null);
                }
            );
            if (disposed) {
                unState();
                unStatus();
            } else {
                unlisteners.push(unState, unStatus);
            }
        };
        setup();

        return () => {
            disposed = true;
            unlisteners.forEach((u) => u());
        };
    }, []);

    // ---- Native watch-to-earn: show Rust's model when no provider is present ----
    // Subscribed only while no provider owns the display, so a provider session
    // costs no listener here.
    const snapshot = useWatchDrops(!externalDropsProvider);
    // This side put a status up, so it is the one to take it down.
    const shownRef = useRef(false);

    useEffect(() => {
        const store = useAppStore.getState();
        const clearShown = () => {
            if (!shownRef.current) return;
            shownRef.current = false;
            store.setLiveDropProgress(null);
            store.setDropProgressActive(false);
            store.setDropProgressComplete(false);
            emit('drop-progress', EMPTY_DROP_STATUS()).catch(() => {});
        };
        // A provider owns the display; stand down once.
        if (externalDropsProvider) {
            clearShown();
            return;
        }
        if (!snapshot) return;
        // The model carries the login; the title bar shows the display name.
        const watching = store.currentStream;
        const displayName = watching?.user_id === snapshot.view.channel_id ? watching?.user_name : undefined;
        const shown = statusFromView(snapshot.view, new Date().toISOString(), displayName);
        if (!shown.status) {
            clearShown();
            return;
        }
        shownRef.current = true;
        store.setDropProgressComplete(shown.complete);
        store.setDropProgressActive(shown.active);
        store.setLiveDropProgress(shown.status);
        emit('drop-progress', shown.status).catch(() => {});
        // Every tier of every campaign earning here, so the Drops page's tier
        // bars and its "next: X in Ym" move with the same numbers as the badge.
        const timestamp = Date.now();
        for (const update of tierUpdates(snapshot.view)) {
            emit('drops-progress-update', { ...update, timestamp }).catch(() => {});
        }
    }, [snapshot, externalDropsProvider]);

    // The farmed stream stopped crediting (offline, or stuck on stale segments
    // the video side cannot tell from live). Rust decides that and numbers each
    // stall; the first window to claim an episode acts on it, once.
    const stallEpisode = !externalDropsProvider && snapshot?.view.stalled ? snapshot.view.stall_episode : 0;
    useEffect(() => {
        if (!stallEpisode) return;
        void (async () => {
            const mine = await invoke<boolean>('ack_drop_stall', { episode: stallEpisode }).catch(() => false);
            if (mine) await handleFarmedStreamStall();
        })();
    }, [stallEpisode]);

    return null;
}

// What a stall does. With auto-switch on, the app's offline auto-switch
// re-verifies the stream is down and moves to another live stream in the same
// category (which re-arms the drop). With it off the stream stays, but an
// offline stream gets a clear cue so earning is not left silently hanging. A
// stream that is still live needs nothing here: Rust opens a new episode if it
// stays stalled.
async function handleFarmedStreamStall(): Promise<void> {
    const store = useAppStore.getState();
    const autoSwitchEnabled = store.settings?.auto_switch?.enabled ?? true;
    if (autoSwitchEnabled) {
        await store.handleStreamOffline();
        return;
    }
    const stream = store.currentStream;
    const userLogin = stream?.user_login;
    if (!userLogin) return;
    try {
        const live = await invoke<unknown | null>('check_stream_online', { userLogin });
        if (live === null) {
            store.addToast(
                `${stream?.user_name || userLogin} went offline, so drop farming paused. Pick another stream to keep earning.`,
                'info',
            );
        }
    } catch (err) {
        Logger.warn('[DropProgressController] offline check failed:', err);
    }
}
