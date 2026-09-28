import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, emit } from '@tauri-apps/api/event';
import { useAppStore } from '../../stores/AppStore';
import type {
    DropProgressStatus,
    DropChannel,
    CurrentDropInfo,
    DropCampaign,
    DropProgress,
    TimeBasedDrop,
    InventoryItem,
    CompletedDrop,
} from '../../types';
import { Logger } from '../../utils/logger';
import { twitchProgressLine } from '../../utils/dropRequirement';

// The single contract id an external drops provider (e.g. an opt-in plugin)
// registers via `plugins_provides` to take over the drop-progress display. The
// core knows nothing else about who provides it. Rename in coordination with
// the provider when its manifest is updated.
const EXTERNAL_DROPS_FEATURE = 'drops.automation';
// The generic status slot a provider pushes its live progress into.
const EXTERNAL_DROPS_STATUS_SLOT = 'drops.status';

// How often to re-read drop progress while watching a drops-enabled stream.
// get_active_drop_campaigns forces a fresh fetch + refreshes the backend progress
// map, and there is no per-minute native progress event, so this poll is what
// keeps the title-bar badge and overlay moving. Twitch credits drops about once a
// minute, so poll a bit faster than that to track it without lag.
const REFRESH_MS = 30_000;

// While farming an active drop, cumulative watch time should climb ~1/min. If
// it stays flat this long, the watched stream has almost certainly gone offline
// (or stopped crediting) — a case the video-side offline detection can miss when
// the low-latency proxy keeps serving stale segments after a stream ends. This
// is the drops-side fallback trigger for the offline auto-switch.
const FARMED_STREAM_STALL_MS = 4 * 60 * 1000;

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
 *  - NATIVE watch-to-earn: when nothing else provides drop progress, this works
 *    out which drop you are earning on the channel you're watching and emits the
 *    'drop-progress' / 'drops-progress-update' events the UI listens for. This is
 *    plain Twitch behaviour (you earn the drop for the stream you watch).
 *  - EXTERNAL provider: when a plugin registers for the generic drops feature, it
 *    takes over — this translates its `drops.status` pushes into the same events,
 *    and the native source stands down. The plugin can progress drops without an
 *    on-screen stream; the core neither knows nor cares how.
 *
 * Either way the same display lights up identically, even while the Drops overlay
 * is closed. `externalDropsProvider` (published to the store) lets the rest of
 * the app render provider-only controls only when a provider is present.
 */
export default function DropProgressController() {
    const currentStream = useAppStore((s) => s.currentStream);
    const externalDropsProvider = useAppStore((s) => s.externalDropsProvider);

    const userId = currentStream?.user_id;
    const userLogin = currentStream?.user_login;
    const displayName = currentStream?.user_name;
    const gameName = currentStream?.game_name;

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

    // ---- Native watch-to-earn: drive the display when no provider is present ----
    const nativeActiveRef = useRef(false);
    // Set when the watched game's campaign is fully earned (nothing left to farm),
    // so the completion status is emitted once rather than every poll.
    const nativeCompleteRef = useRef(false);
    // The game we last showed progress for, so the keep-last guard only holds
    // within the SAME stream/game (a real game switch clears stale progress).
    const lastGameRef = useRef<string | undefined>(undefined);
    // Offline/stall watchdog for the farmed stream (see FARMED_STREAM_STALL_MS):
    // the highest cumulative watch time seen, when it last advanced, and whether
    // this stall episode has already been acted on (so we react once per stall).
    const lastCumulativeRef = useRef(-1);
    const lastAdvanceAtRef = useRef(0);
    const stallHandledRef = useRef(false);

    useEffect(() => {
        let disposed = false;
        let timer: ReturnType<typeof setInterval> | null = null;

        // A drops campaign is ONE cumulative watch-time counter with reward tiers
        // (e.g. 15 / 30 / 60 min). The reward you're "on" is the lowest tier you
        // haven't reached yet, and its progress is the cumulative time over that
        // tier's threshold. Twitch zeroes a tier's own counter once it's
        // completed/claimed, so we read the cumulative ACROSS the campaign (the
        // most any tier reports, and at least the threshold of any claimed tier)
        // rather than trusting a single tier's possibly-0 counter — otherwise we
        // land on a finished early tier and show 0%.
        const pickCurrentDrop = (
            campaign: DropCampaign,
            progressArray: DropProgress[],
            inventoryItems: InventoryItem[],
            completedDrops: CompletedDrop[],
        ): { drop: CurrentDropInfo; cumulative: number; sourceDrops: TimeBasedDrop[] } | null => {
            // A fresh campaigns fetch carries instance drop ids that match nothing
            // in the progress map / inventory, so its tiers all resolve to 0. The
            // inventory item for this campaign is internally consistent (its drop
            // ids and minutes agree, and match the progress map), so use ITS drops
            // as the source of truth — same as the overlay.
            const invItem = inventoryItems.find(
                (it) => it.campaign.id === campaign.id ||
                    it.campaign.name.toLowerCase() === campaign.name.toLowerCase() ||
                    it.campaign.game_name?.toLowerCase() === campaign.game_name?.toLowerCase(),
            );
            const drops = invItem?.campaign.time_based_drops?.length
                ? invItem.campaign.time_based_drops
                : campaign.time_based_drops;
            // The inventory drop's OWN embedded progress carries the real accrued
            // minutes. get_drop_progress is all-zero in this build, and it has a
            // (zero) entry per drop, so checking it first would mask the real
            // value — prefer the inventory's embedded progress.
            const progressOf = (d: TimeBasedDrop): DropProgress | null =>
                d.progress || progressArray.find((p) => p.drop_id === d.id) || null;

            // A tier you ALREADY OWN must be excluded, or the badge parks at 0% on a
            // stream with nothing left to earn. The active fetch only sets is_claimed
            // on the live instance, but Twitch reissues a campaign under a new instance
            // (fresh drop/benefit ids, is_claimed=false, 0 minutes) while the reward
            // still sits in your permanent collection. So mirror the Drops overlay's
            // ownership rule: claimed here, OR the reward is in completed_drops / a
            // claimed inventory drop — matched by benefit id AND by benefit NAME, since
            // reissues mint new ids. A tier with live watch-time is a genuine fresh
            // in-progress drop and is never treated as owned.
            const ownedBenefitIds = new Set<string>(completedDrops.map((d) => d.id));
            const ownedBenefitNames = new Set<string>(
                completedDrops.map((d) => (d.name || '').toLowerCase().trim()).filter(Boolean),
            );
            const ownedDropIds = new Set<string>();
            for (const item of inventoryItems) {
                for (const d of item.campaign.time_based_drops) {
                    if (d.progress?.is_claimed !== true) continue;
                    ownedDropIds.add(d.id);
                    d.benefit_edges?.forEach((b) => {
                        ownedBenefitIds.add(b.id);
                        if (b.name) ownedBenefitNames.add(b.name.toLowerCase().trim());
                    });
                }
            }
            const isOwned = (d: TimeBasedDrop): boolean => {
                const p = progressOf(d);
                if (p?.is_claimed === true) return true;
                if ((p?.current_minutes_watched ?? 0) > 0) return false;
                if (ownedDropIds.has(d.id)) return true;
                return !!d.benefit_edges?.some(
                    (b) =>
                        ownedBenefitIds.has(b.id) ||
                        (!!b.name && ownedBenefitNames.has(b.name.toLowerCase().trim())),
                );
            };

            let cumulative = 0;
            for (const d of drops) {
                const p = progressOf(d);
                cumulative = Math.max(cumulative, p?.current_minutes_watched ?? 0);
                if (isOwned(d)) cumulative = Math.max(cumulative, d.required_minutes_watched ?? 0);
            }
            const tier = drops
                .filter((d) => (d.required_minutes_watched ?? 0) > 0 && !isOwned(d))
                .sort((a, b) => (a.required_minutes_watched ?? 0) - (b.required_minutes_watched ?? 0))
                .find((d) => (d.required_minutes_watched ?? 0) > cumulative);
            if (!tier) return null;
            const required = tier.required_minutes_watched ?? 0;
            return {
                cumulative,
                sourceDrops: drops,
                drop: {
                    campaign_id: campaign.id,
                    campaign_name: campaign.name,
                    drop_id: tier.id,
                    drop_name: tier.benefit_edges?.[0]?.name || tier.name || '',
                    drop_image: tier.benefit_edges?.[0]?.image_url,
                    required_minutes: required,
                    current_minutes: Math.min(cumulative, required),
                    game_name: campaign.game_name,
                },
            };
        };

        const clearNative = () => {
            if (!nativeActiveRef.current && !nativeCompleteRef.current) return;
            nativeActiveRef.current = false;
            nativeCompleteRef.current = false;
            lastCumulativeRef.current = -1;
            lastAdvanceAtRef.current = 0;
            stallHandledRef.current = false;
            const store = useAppStore.getState();
            store.setLiveDropProgress(null);
            store.setDropProgressActive(false);
            store.setDropProgressComplete(false);
            emit('drop-progress', {
                active: false,
                complete: false,
                current_channel: null,
                current_campaign: null,
                current_drop: null,
                eligible_channels: [],
                last_update: new Date().toISOString(),
            } as DropProgressStatus).catch(() => {});
        };

        // The watched game's campaign has every watch-time reward earned — nothing
        // left to farm. Emit a completion status (active:false, complete:true) so
        // the title-bar badge shows a "done" check instead of reverting to the
        // idle gift, and the overlay reflects it. Idempotent across polls.
        const markNativeComplete = (channel: DropChannel, campaignId: string) => {
            nativeActiveRef.current = false;
            const store = useAppStore.getState();
            const status: DropProgressStatus = {
                active: false,
                complete: true,
                current_channel: channel,
                current_campaign: campaignId,
                current_drop: null,
                eligible_channels: [],
                last_update: new Date().toISOString(),
            };
            store.setLiveDropProgress(status);
            store.setDropProgressActive(false);
            store.setDropProgressComplete(true);
            if (!nativeCompleteRef.current) emit('drop-progress', status).catch(() => {});
            nativeCompleteRef.current = true;
        };

        // Fallback reaction when the farmed stream stalls (offline). Reuses the
        // app's existing offline auto-switch when it's enabled — that re-verifies
        // the stream is down, moves to another live stream in the SAME category
        // (which re-arms the drop), and notifies. When auto-switch is off we don't
        // move the stream, but still confirm offline and surface a clear cue so
        // farming isn't left silently hanging with no indication.
        const handleFarmedStreamStall = async () => {
            const store = useAppStore.getState();
            const autoSwitchEnabled = store.settings?.auto_switch?.enabled ?? true;
            if (autoSwitchEnabled) {
                store.handleStreamOffline();
                return;
            }
            try {
                const live = await invoke<unknown | null>('check_stream_online', { userLogin });
                if (live === null) {
                    store.addToast(
                        `${displayName ?? userLogin ?? 'Stream'} went offline — drop farming paused. Pick another stream to keep earning.`,
                        'info',
                    );
                } else {
                    // Still live (just slow to credit): allow re-detection.
                    stallHandledRef.current = false;
                }
            } catch {
                stallHandledRef.current = false;
            }
        };

        const refresh = async () => {
            // A provider owns the display; or nothing is being watched.
            if (externalDropsProvider || !userId || !gameName) {
                clearNative();
                return;
            }
            // A real game switch must clear stale progress immediately. Within the
            // SAME game, transient empty fetches keep the last good value so the
            // badge never blinks off between polls.
            const gameChanged = lastGameRef.current !== gameName;
            lastGameRef.current = gameName;
            if (gameChanged) {
                // New stream/game: restart the stall watchdog from scratch.
                lastCumulativeRef.current = -1;
                lastAdvanceAtRef.current = 0;
                stallHandledRef.current = false;
            }
            try {
                const campaigns = await invoke<DropCampaign[]>('get_active_drop_campaigns').catch(() => null);
                if (disposed) return;
                if (!campaigns || campaigns.length === 0) { if (gameChanged) clearNative(); return; }

                // Match the campaign by the category being watched: you earn the drop
                // for the game the channel is actually streaming. Do NOT match an ACL
                // campaign by channel alone — an allow-listed streamer playing a
                // different game (e.g. on the list for a Marvel Rivals drop but live in
                // Counter-Strike) is not earning it, so showing it would be wrong.
                const game = gameName.toLowerCase();
                const rawMatched = campaigns.find((c) => c.game_name?.toLowerCase() === game) ?? null;
                if (!rawMatched) {
                    // No drop for this category, but a drop that runs across
                    // categories (multi-day ones) may still be earning here. Rust
                    // decides that from Twitch's own minutes going up (accruing);
                    // this only shows it.
                    const live = (await invoke<DropProgress[]>('get_drop_progress').catch(() => [] as DropProgress[]))
                        .find((p) => p.twitch_progress?.accruing && !p.is_claimed);
                    if (disposed) return;
                    const liveCampaign = live ? campaigns.find((c) => c.id === live.campaign_id) : undefined;
                    const liveDrop = liveCampaign?.time_based_drops.find((d) => d.id === live?.drop_id);
                    if (live && liveCampaign && liveDrop) {
                        const status: DropProgressStatus = {
                            active: true,
                            current_channel: {
                                id: userId,
                                name: userLogin ?? '',
                                display_name: displayName ?? userLogin ?? '',
                                game_name: gameName,
                                viewer_count: 0,
                                is_live: true,
                                drops_enabled: true,
                            },
                            current_campaign: liveCampaign.id,
                            current_drop: {
                                campaign_id: liveCampaign.id,
                                campaign_name: liveCampaign.name,
                                drop_id: liveDrop.id,
                                drop_name: liveDrop.name,
                                drop_image: liveDrop.benefit_edges?.[0]?.image_url,
                                required_minutes: live.required_minutes_watched,
                                current_minutes: live.current_minutes_watched,
                                game_name: liveCampaign.game_name,
                                detail: twitchProgressLine(live, liveDrop) ?? undefined,
                            },
                            eligible_channels: [],
                            last_update: new Date().toISOString(),
                        };
                        nativeActiveRef.current = true;
                        nativeCompleteRef.current = false;
                        emit('drop-progress', status).catch(() => {});
                        emit('drops-progress-update', {
                            drop_id: live.drop_id,
                            current_minutes: live.current_minutes_watched,
                            required_minutes: live.required_minutes_watched,
                            timestamp: Date.now(),
                            campaign_id: live.campaign_id,
                        }).catch(() => {});
                        const store = useAppStore.getState();
                        store.setDropProgressComplete(false);
                        store.setDropProgressActive(true);
                        store.setLiveDropProgress(status);
                        return;
                    }
                    // Nothing earning: clear a meter this branch put up, and a
                    // stale one on a game switch.
                    if (gameChanged || nativeActiveRef.current) clearNative();
                    return;
                }

                // The campaign's own embedded progress reads 0; the real per-tier
                // minutes live in the progress map + inventory. Merge them in the
                // exact same way the Drops overlay does so the badge matches it.
                const progress = await invoke<DropProgress[]>('get_drop_progress').catch(() => [] as DropProgress[]);
                const inventory = await invoke<{ items?: InventoryItem[]; completed_drops?: CompletedDrop[] }>('get_drops_inventory').catch(() => null);
                if (disposed) return;
                const matched = rawMatched;
                const picked = pickCurrentDrop(matched, progress, inventory?.items ?? [], inventory?.completed_drops ?? []);
                // Campaign present but every tier is reached/claimed. If the campaign
                // actually had watch-time rewards, that's DONE (all farmed) — surface
                // it as complete so the badge/overlay stop reading as in-progress.
                // A campaign with no watch-time rewards has simply nothing to show.
                if (!picked) {
                    const hasWatchTimeReward = matched.time_based_drops.some((d) => (d.required_minutes_watched ?? 0) > 0);
                    if (hasWatchTimeReward) {
                        const doneChannel: DropChannel = {
                            id: userId,
                            name: userLogin ?? '',
                            display_name: displayName ?? userLogin ?? '',
                            game_name: gameName,
                            viewer_count: 0,
                            is_live: true,
                            drops_enabled: true,
                        };
                        markNativeComplete(doneChannel, matched.id);
                    } else {
                        clearNative();
                    }
                    return;
                }
                const drop = picked.drop;
                const cumulative = picked.cumulative;

                // Cumulative only grows; a 0 after we were already showing progress
                // means a transient empty progress/inventory fetch this tick — keep
                // the last good badge rather than dropping to 0%. (On a fresh stream
                // nativeActiveRef is false, so a genuine 0%-start still shows.)
                if (cumulative === 0 && nativeActiveRef.current && !gameChanged) return;

                // Offline/stall watchdog: while an active drop is being farmed the
                // cumulative time should keep climbing. If it advanced, (re)start
                // the clock; if it's been flat past the threshold, react once
                // (auto-switch to another live channel / surface an offline cue).
                if (cumulative > 0) {
                    if (cumulative > lastCumulativeRef.current || lastAdvanceAtRef.current === 0) {
                        lastAdvanceAtRef.current = Date.now();
                        stallHandledRef.current = false;
                    } else if (
                        nativeActiveRef.current &&
                        !stallHandledRef.current &&
                        Date.now() - lastAdvanceAtRef.current >= FARMED_STREAM_STALL_MS
                    ) {
                        stallHandledRef.current = true;
                        void handleFarmedStreamStall();
                    }
                    lastCumulativeRef.current = Math.max(lastCumulativeRef.current, cumulative);
                }

                const channel: DropChannel = {
                    id: userId,
                    name: userLogin ?? '',
                    display_name: displayName ?? userLogin ?? '',
                    game_name: gameName,
                    viewer_count: 0,
                    is_live: true,
                    drops_enabled: true,
                };
                const status: DropProgressStatus = {
                    active: true,
                    current_channel: channel,
                    current_campaign: drop.campaign_id,
                    current_drop: drop,
                    eligible_channels: [],
                    last_update: new Date().toISOString(),
                };
                nativeActiveRef.current = true;
                // A fresh in-progress drop supersedes any earlier "done" state (e.g. a
                // new reward tier or campaign appeared for this game).
                nativeCompleteRef.current = false;
                useAppStore.getState().setDropProgressComplete(false);
                emit('drop-progress', status).catch(() => {});
                // Push fresh progress for EVERY tier of the watched campaign, each
                // as the cumulative time capped at that tier's threshold (so lower
                // tiers read full, the current tier partial, higher tiers their
                // share). This keeps the overlay's per-tier bars and the badge in
                // sync and derived from the same cumulative truth.
                const now = Date.now();
                for (const d of picked.sourceDrops) {
                    const req = d.required_minutes_watched ?? 0;
                    if (req <= 0) continue;
                    emit('drops-progress-update', {
                        drop_id: d.id,
                        current_minutes: Math.min(cumulative, req),
                        required_minutes: req,
                        timestamp: now,
                        campaign_id: matched.id,
                    }).catch(() => {});
                }
                useAppStore.getState().setDropProgressActive(true);
                useAppStore.getState().setLiveDropProgress(status);
            } catch (err) {
                Logger.warn('[DropProgressController] native refresh failed:', err);
            }
        };

        refresh();
        timer = setInterval(refresh, REFRESH_MS);

        return () => {
            disposed = true;
            if (timer) clearInterval(timer);
            // NOTE: deliberately do NOT clearNative() here. The effect re-runs on
            // benign dep churn (e.g. the watched-stream object refreshing), and a
            // clear here would blink the badge to 0% every cycle. Clears are driven
            // by refresh() instead (no stream, or a real game switch via gameChanged).
        };
    }, [userId, userLogin, displayName, gameName, externalDropsProvider]);

    return null;
}
