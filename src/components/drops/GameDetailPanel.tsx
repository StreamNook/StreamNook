import { useState, useRef } from 'react';
import { X, Gift, Package, Check, Pause, Clock, Star, Ban, Link2, Tv } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../stores/AppStore';
import type { UnifiedGame, DropProgress, DropProgressStatus, DropCampaign, TimeBasedDrop, InventoryItem, CompletedDrop, DropBenefit, TwitchStream } from '../../types';
import { Tooltip } from '../ui/Tooltip';
import { usePluginUiRegistry, selectSlot } from '../../plugins-ui/registry';
import { DROPS_CARD_ACTION_SLOT, type DropCardActionContext, type PickedDropChannel } from '../../plugins-ui/types';
import ChannelPickerModal, { type PickableChannel } from './ChannelPickerModal';

import { Logger } from '../../utils/logger';
// Helper to check if a drop is collectible
// Uses the is_collectible field from backend, with fallback to checking required_minutes_watched
// Also checks inventory data as a secondary source since it has more accurate progress info
function isDropCollectible(drop: TimeBasedDrop, inventoryItems?: InventoryItem[]): boolean {
    // If is_collectible is explicitly set, use it
    if (typeof drop.is_collectible === 'boolean') {
        return drop.is_collectible;
    }
    
    // Check if required_minutes_watched is set and > 0
    if (drop.required_minutes_watched > 0) {
        return true;
    }
    
    // Fallback: Check inventory items for this drop's data
    // Inventory data often has more accurate required_minutes_watched values
    if (inventoryItems && inventoryItems.length > 0) {
        for (const item of inventoryItems) {
            const inventoryDrop = item.campaign.time_based_drops.find(d => d.id === drop.id);
            if (inventoryDrop) {
                // Check inventory drop's is_collectible
                if (typeof inventoryDrop.is_collectible === 'boolean') {
                    return inventoryDrop.is_collectible;
                }
                // Check inventory drop's required_minutes_watched
                if (inventoryDrop.required_minutes_watched > 0) {
                    return true;
                }
                // Check inventory drop's progress.required_minutes_watched
                if (inventoryDrop.progress && inventoryDrop.progress.required_minutes_watched > 0) {
                    return true;
                }
            }
        }
    }
    
    // Check if the drop has progress data with required_minutes
    if (drop.progress && drop.progress.required_minutes_watched > 0) {
        return true;
    }
    
    // Default: not collectible if we can't determine watch time requirement
    return false;
}

// Helper to check if a campaign is collectible
// A campaign is collectible if it has at least one collectible time_based_drop
function isCampaignCollectible(campaign: DropCampaign, inventoryItems?: InventoryItem[]): boolean {
    if (!campaign.time_based_drops || campaign.time_based_drops.length === 0) {
        return false;
    }
    // Campaign is collectible if ANY of its drops are collectible
    return campaign.time_based_drops.some(drop => isDropCollectible(drop, inventoryItems));
}

// Get the drop type label for a campaign
function getCampaignDropType(campaign: DropCampaign, inventoryItems?: InventoryItem[]): { type: 'time' | 'instant' | 'mixed' | 'other'; label: string } {
    if (!campaign.time_based_drops || campaign.time_based_drops.length === 0) {
        return { type: 'other', label: 'Event/Special' };
    }
    
    const collectibleCount = campaign.time_based_drops.filter(d => isDropCollectible(d, inventoryItems)).length;
    const nonCollectibleCount = campaign.time_based_drops.length - collectibleCount;
    
    if (collectibleCount > 0 && nonCollectibleCount > 0) {
        return { type: 'mixed', label: 'Mixed' };
    } else if (collectibleCount > 0) {
        return { type: 'time', label: 'Watch Time' };
    } else {
        return { type: 'instant', label: 'Event/Special' };
    }
}

// "How do I unlock this" text for a reward that can't be earned by watch time.
// Twitch exposes no structured reason (subscription vs gift vs event), so we infer
// it from the most reliable signals we DO have. The reward name is often the
// clearest tell (e.g. "Gifted Sub Drop"), so it's checked first; the campaign
// description is the next-best source, used verbatim when it carries real text.
// Falls back to a generic note when nothing is informative.
function unlockRequirementText(rewardName?: string, description?: string): string {
    const desc = (description || '').replace(/\s+/g, ' ').trim();
    const haystack = `${rewardName || ''} ${desc}`.toLowerCase();

    // Match a known unlock condition by keyword. Order matters: "gifted sub" is
    // more specific than a plain sub, and Prime is a distinct sub flavor.
    const mentionsSub = /\bsub(s|scribe|scriber|scription)?\b/.test(haystack);
    if (/\bgift/.test(haystack) && mentionsSub) {
        return 'Gift a subscription in a participating channel to unlock this reward.';
    }
    if (/\bprime\b/.test(haystack)) {
        return 'Subscribe with Prime in a participating channel to unlock this reward.';
    }
    if (mentionsSub) {
        return 'Subscribe to a participating channel to unlock this reward.';
    }
    if (/\b(cheer|bits)\b/.test(haystack)) {
        return 'Cheer bits in a participating channel to unlock this reward.';
    }
    if (/\bfollow/.test(haystack)) {
        return 'Follow a participating channel to unlock this reward.';
    }

    // No recognizable condition: show the campaign's own description if it has any
    // real text, otherwise a generic note.
    if (desc) return desc;
    return "Can't be earned by watching. This reward unlocks through a subscription, gift, or special action for its campaign.";
}

// Match a reward's benefit name against the user's earned badge titles. Badge drops
// rarely land in the permanent gameEventDrops list, so a held badge title is the most
// reliable "you already have this" signal for them. Matched EXACTLY: a looser prefix
// match conflated distinct rewards from the same campaign, because every reward in a
// campaign shares the game-name prefix (e.g. a watch-earned "Two Point Museum" badge
// would mark an unearned "Two Point Museum" subscriber badge as owned). Reissues that
// carry the same name are still caught here; renamed reissues fall to benefit-id/name
// matching at the call sites. Single source of truth: the rewards tally, Active
// Campaigns, and Your Collection all call this, so they agree on ownership.
function matchesEarnedBadge(benefitName: string | undefined, earnedBadgeTitles: Set<string>): boolean {
    const bn = (benefitName || '').toLowerCase().trim();
    if (!bn) return false;
    return earnedBadgeTitles.has(bn);
}

type RewardKind = 'badge' | 'emote' | 'item';

// Classify a reward for display. The signals, in order of reliability:
//  - The art's CDN path: '/emoticons/' = emote, '/badges/' = badge.
//  - Twitch flags chat badges via distribution_type ('BADGE'). Game drops are
//    'DIRECT_ENTITLEMENT' whether emote or in-game item, so that can't split those two.
//  - The name ("...Emote" / "...Badge").
//  - The decider for the hard case: whether the reward name is in Twitch's badge catalog
//    (knownBadgeTitles = earned + global badges). A global badge like "YOU GOT THIS" and a
//    real in-game item can BOTH carry a null type and the same generic quests asset URL,
//    so the catalog is the only thing that tells them apart.
// Genuine in-game items (not in the badge catalog, no cosmetic signal) fall through to 'item'.
function getRewardKind(benefit: DropBenefit | undefined, knownBadgeTitles: Set<string>): RewardKind {
    const name = (benefit?.name || '').toLowerCase().trim();
    const img = (benefit?.image_url || '').toLowerCase();
    // Emote first: quest emotes share the generic quests asset bucket with badges.
    if (img.includes('/emoticons/') || name.includes('emote')) return 'emote';
    if (benefit?.distribution_type === 'BADGE') return 'badge';
    if (img.includes('/badges/') || name.includes('badge')) return 'badge';
    if (name && knownBadgeTitles.has(name)) return 'badge';
    return 'item';
}

function rewardKindLabel(kind: RewardKind): string {
    return kind === 'badge' ? 'Badge' : kind === 'emote' ? 'Emote' : 'Item';
}

interface GameDetailPanelProps {
    game: UnifiedGame;
    allGames: UnifiedGame[]; // All games for global drop metadata lookup
    progress: DropProgress[];
    completedDrops: CompletedDrop[]; // List of all completed drops from inventory
    earnedBadgeTitles: Set<string>; // Set of earned badge titles (lowercase) for name-based ownership matching
    knownBadgeTitles: Set<string>; // Earned + global badge catalog titles (lowercase) for reward-kind labeling
    dropProgress: DropProgressStatus | null;

    isOpen: boolean;
    onClose: () => void;
    onStopAutomation: () => void;
    onClaimDrop: (dropId: string, dropInstanceId?: string) => void;
    onWatchChannel: (channelLogin: string, streamInfo?: TwitchStream) => void; // open a channel in the player to watch (native earn)
}

// Helper to merge progress from inventory into campaigns
// This ensures we show the most accurate progress data even if the progress array doesn't have it
export function mergeProgressFromInventory(
    campaign: DropCampaign,
    inventoryItems: InventoryItem[],
    progressArray: DropProgress[]
): DropCampaign {
    // Find matching inventory item for this campaign
    const inventoryItem = inventoryItems.find(item => 
        item.campaign.id === campaign.id ||
        item.campaign.name.toLowerCase() === campaign.name.toLowerCase()
    );
    
    if (!inventoryItem) return campaign;
    
    // Merge progress from inventory into each drop
    const mergedDrops = campaign.time_based_drops.map(drop => {
        // First check progress array (real-time updates take priority)
        const progressEntry = progressArray.find(p => p.drop_id === drop.id);
        if (progressEntry) {
            return {
                ...drop,
                progress: progressEntry,
            };
        }
        
        // Then check inventory item for this drop's progress
        const inventoryDrop = inventoryItem.campaign.time_based_drops.find(d => d.id === drop.id);
        if (inventoryDrop?.progress) {
            return {
                ...drop,
                progress: inventoryDrop.progress,
                // Also copy over required_minutes_watched from inventory if our drop has 0
                required_minutes_watched: drop.required_minutes_watched || inventoryDrop.required_minutes_watched,
                is_collectible: drop.is_collectible ?? (inventoryDrop.required_minutes_watched > 0),
            };
        }
        
        // Use existing drop progress or keep as-is
        return drop;
    });
    
    return {
        ...campaign,
        time_based_drops: mergedDrops,
    };
}

export default function GameDetailPanel({
    game,
    allGames,
    progress,
    completedDrops,
    earnedBadgeTitles,
    knownBadgeTitles,
    dropProgress,

    isOpen,
    onClose,
    onStopAutomation,
    onClaimDrop,
    onWatchChannel,
}: GameDetailPanelProps) {
    // A provider (opt-in plugin) is present: only then is there anything to
    // "stop". Native watch-to-earn is stopped simply by not watching.
    const externalDropsProvider = useAppStore((s) => s.externalDropsProvider);

    // Channel picker shared by core and the automation plugin. Core picks a channel to
    // WATCH; the plugin (via pickChannel) picks one to collect. A pending resolver lets
    // the plugin's pickChannel() await the user's choice.
    const [picker, setPicker] = useState<{ campaign: DropCampaign; actionLabel: string; onPick: (c: PickableChannel) => void } | null>(null);
    const pickResolverRef = useRef<((c: PickedDropChannel | null) => void) | null>(null);

    const openWatchPicker = (campaign: DropCampaign) => {
        setPicker({
            campaign,
            actionLabel: 'Watch',
            onPick: (c) => { setPicker(null); onWatchChannel(c.login, c.stream); },
        });
    };

    const requestPickChannel = (campaign: DropCampaign): Promise<PickedDropChannel | null> => {
        return new Promise((resolve) => {
            pickResolverRef.current = resolve;
            setPicker({
                campaign,
                actionLabel: 'Collect',
                onPick: (c) => {
                    setPicker(null);
                    const r = pickResolverRef.current; pickResolverRef.current = null;
                    r?.({ login: c.login, displayName: c.displayName, userId: c.userId });
                },
            });
        });
    };

    const closePicker = () => {
        setPicker(null);
        const r = pickResolverRef.current; pickResolverRef.current = null;
        r?.(null);
    };
    // Merge inventory progress into active campaigns for accurate display
    const campaignsWithMergedProgress = game.active_campaigns.map(campaign =>
        mergeProgressFromInventory(campaign, game.inventory_items, progress)
    );

    // Rewards/drops the user has genuinely EARNED, used to mark a reward as already
    // owned when the active campaign instance hasn't synced the claim. Built from
    // unambiguous sources: the permanent gameEventDrops list (completedDrops) and any
    // inventory drop explicitly flagged is_claimed. Benefit NAMES are collected too,
    // but name matching is applied to BADGE rewards only (see isRewardOwned): badges
    // rarely appear in completedDrops and can never be re-earned, while a consumable
    // reward reissued under a new campaign (same name, new ids) is earnable again.
    const ownedBenefitIds = new Set<string>(completedDrops.map(d => d.id));
    const ownedBenefitNames = new Set<string>(
        completedDrops.map(d => (d.name || '').toLowerCase().trim()).filter(Boolean)
    );
    const ownedDropIds = new Set<string>();
    game.inventory_items.forEach(item => {
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
    // A reward counts as already-owned ONLY when this drop isn't being actively collected
    // here. If it has its own claim, that wins. Otherwise cross-instance matching (by
    // drop id / benefit id / benefit name / earned badge title) applies ONLY when the
    // drop has no current progress: a drop you have watch-time on is a fresh in-progress
    // drop and must never be treated as owned just because a same-named reward was earned
    // elsewhere.
    const isRewardOwned = (drop: TimeBasedDrop, dp?: DropProgress | null): boolean => {
        if (dp?.is_claimed === true) return true;
        // is_claimed handled above; only watch-time counts as "in progress" here.
        const hasCurrentProgress = !!dp && (dp.current_minutes_watched || 0) > 0;
        if (hasCurrentProgress) return false;
        if (ownedDropIds.has(drop.id)) return true;
        if (drop.benefit_edges?.some(b => ownedBenefitIds.has(b.id))) return true;
        // Name matching is badge-only. A held badge can't be earned again, but a
        // consumable reward reissued under a new campaign instance (same name, new
        // benefit ids, e.g. recurring game currency) is genuinely earnable again;
        // counting a name match as ownership filed live campaigns under a false
        // "Completed" while their cards still offered Mine at 0 minutes.
        const benefit = drop.benefit_edges?.[0];
        if (getRewardKind(benefit, knownBadgeTitles) !== 'badge') return false;
        if (!!benefit?.name && ownedBenefitNames.has(benefit.name.toLowerCase().trim())) return true;
        // Badges rarely appear in completedDrops, but they show up in the user's
        // earned badge titles.
        return matchesEarnedBadge(benefit?.name, earnedBadgeTitles);
    };

    // Union of every drop id we can prove is already earned, matching how the
    // Active Campaigns filter decides ownership: this game's claimed inventory,
    // plus real-time claims from the progress array, plus claimed drops from
    // EVERY game's inventory (catches rewards from expired campaigns that the
    // backend's completed list has dropped).
    const earnedDropIds = new Set<string>(ownedDropIds);
    progress.forEach(p => { if (p.is_claimed === true) earnedDropIds.add(p.drop_id); });
    allGames.forEach(g => g.inventory_items.forEach(item => item.campaign.time_based_drops.forEach(d => {
        if (d.progress?.is_claimed === true) earnedDropIds.add(d.id);
    })));

    // THE single ownership rule. The rewards tally, the Active and Completed
    // filters, and every CampaignCard all judge a drop through this one
    // predicate, so a card can never disagree with the section it sits in (a
    // "Completed" campaign once kept a live Mine button at 0 minutes because
    // the card matched ownership by benefit id while the section matched by
    // name).
    const isDropOwned = (drop: TimeBasedDrop, dp?: DropProgress | null): boolean =>
        isRewardOwned(drop, dp) || earnedDropIds.has(drop.id);

    // "This watch-time reward is still collectible": not owned and not yet
    // 100% watched. The Completed Campaigns section is its precise complement,
    // so a campaign always lands in exactly one section.
    const isDropEarnable = (drop: TimeBasedDrop): boolean => {
        const dp = drop.progress || progress.find(p => p.drop_id === drop.id) || null;
        if (isDropOwned(drop, dp)) return false;
        const required = dp?.required_minutes_watched || drop.required_minutes_watched || 0;
        if (required <= 0) return false; // reward isn't earned by watching
        return (dp?.current_minutes_watched || 0) < required; // not yet 100% → still earnable
    };
    // Check if automation this game
    // Use current_drop.game_name OR current_channel.game_name as fallback (current_drop may not be set immediately)
    const isProgressingThisGame = dropProgress?.active && (
        dropProgress.current_drop?.game_name?.toLowerCase() === game.name?.toLowerCase() ||
        dropProgress.current_channel?.game_name?.toLowerCase() === game.name?.toLowerCase()
    );

    // Transform box art URL to higher resolution
    // GQL API returns URLs with fixed dimensions (e.g., "52x72"), not placeholders
    // Helix API returns URLs with {width}x{height} placeholders
    // We need to handle both cases
    const getHighResBoxArt = (url: string | undefined): string => {
        if (!url) return 'https://static-cdn.jtvnw.net/ttv-static/404_boxart-288x384.jpg';

        // If URL has placeholders (Helix style), replace them
        if (url.includes('{width}') && url.includes('{height}')) {
            return url.replace('{width}', '288').replace('{height}', '384');
        }

        // If URL has fixed dimensions (GQL style), replace them with high res
        // Pattern: -WIDTHxHEIGHT.jpg or -WIDTHxHEIGHT.png
        const fixedDimensionPattern = /-\d+x\d+\.(jpg|png|jpeg|webp)/i;
        if (fixedDimensionPattern.test(url)) {
            return url.replace(fixedDimensionPattern, '-288x384.$1');
        }

        // Fallback: return as-is
        return url;
    };

    const boxArtUrl = getHighResBoxArt(game.box_art_url);

    // Calculate automation progress for this game
    let collectProgress = 0;
    let collectDropName = '';

    let collectDropImage = '';
    let collectBenefitName = '';
    let collectCurrentMins = 0;
    let collectRequiredMins = 0;

    if (isProgressingThisGame && dropProgress?.current_drop) {
        const { drop_id, drop_name } = dropProgress.current_drop;
        const liveProgress = progress.find(p => p.drop_id === drop_id);

        collectCurrentMins = liveProgress ? liveProgress.current_minutes_watched : (dropProgress.current_drop.current_minutes ?? 0);
        collectRequiredMins = liveProgress ? liveProgress.required_minutes_watched : (dropProgress.current_drop.required_minutes ?? 1);

        collectProgress = collectRequiredMins > 0 ? (collectCurrentMins / collectRequiredMins) * 100 : 0;
        collectDropName = drop_name || '';


        // Find the actual drop object to get its benefit image
        const collectDrop = game.active_campaigns
            .flatMap(c => c.time_based_drops)
            .find(d => d.id === drop_id);

        if (collectDrop?.benefit_edges?.[0]) {
            collectDropImage = collectDrop.benefit_edges[0].image_url || '';
            collectBenefitName = collectDrop.benefit_edges[0].name || collectDropName;
        }
    }

    if (!isOpen) return null;

    return (
        <>
        <div className="fixed inset-0 z-50 flex items-center justify-center p-4 sm:p-6">
            {/* Backdrop */}
            <div
                className="absolute inset-0 bg-black/60 backdrop-blur-sm animate-in fade-in duration-200"
                onClick={onClose}
            />

            {/* Centered modal shell. Matches the app's modal primitive (cf. ChannelPickerModal). */}
            <div className="relative w-full max-w-lg max-h-[85vh] bg-background rounded-xl shadow-2xl border border-borderLight overflow-hidden flex flex-col animate-in fade-in zoom-in-95 duration-200">
                {/* Header */}
                <div className="flex items-center gap-3 px-5 py-4 border-b border-borderSubtle bg-backgroundSecondary">
                    <img
                        src={boxArtUrl}
                        alt={game.name}
                        className="w-20 aspect-[3/4] rounded-lg object-cover border border-borderLight shadow-md"
                    />
                    <div className="flex-1 min-w-0">
                        <h3 className="font-bold text-textPrimary text-base truncate">
                            {game.name}
                        </h3>
                        <p className="text-xs text-textSecondary mt-0.5">
                            {game.active_campaigns.length} campaign{game.active_campaigns.length !== 1 ? 's' : ''} active
                        </p>
                        {(() => {
                            // Twitch marks a campaign as needing an account link (accountLinkURL)
                            // that credits drops only once the viewer connects; self.isAccountConnected
                            // mirrors what Twitch's own rewards page shows. We surface the connect
                            // action only when Twitch reports not-connected (same rule as Twitch), and
                            // show nothing once it's satisfied — no permanent stamp.
                            const connectUrl = game.active_campaigns
                                .find(c => c.account_link && !c.is_account_connected)?.account_link;
                            if (!connectUrl) return null;
                            return (
                                <Tooltip content="Open the account-connection page in your browser, where you're already signed in to Twitch and the game.">
                                    <button
                                        onClick={(e) => {
                                            e.stopPropagation();
                                            // The link page is a publisher-side flow (game login + Twitch
                                            // OAuth), not a twitch.tv-cookie op, so it opens in the real
                                            // browser rather than the in-app Twitch overlay. Signal the
                                            // Drops Center to re-check status when the app regains focus.
                                            invoke('open_browser_url', { url: connectUrl }).catch(() => {});
                                            window.dispatchEvent(new CustomEvent('drops-connect-initiated'));
                                        }}
                                        className="glass-button px-2.5 py-1 mt-1 text-xs font-semibold text-accent flex items-center gap-1.5"
                                    >
                                        <Link2 size={12} />
                                        Connect account
                                    </button>
                                </Tooltip>
                            );
                        })()}
                    </div>
                    <button
                        onClick={onClose}
                        className="p-2 text-textSecondary hover:text-textPrimary hover:bg-surface rounded-lg transition-all"
                    >
                        <X size={18} />
                    </button>
                </div>

                {/* Content - Scrollable */}
                <div className="flex-1 overflow-y-auto custom-scrollbar p-4 space-y-6">
                    {/* Rewards showcase: every reward this game's active campaigns offer, with your progress */}
                    {(() => {
                        const seen = new Set<string>();
                        const rewards = campaignsWithMergedProgress
                            // Keep each drop paired with its campaign description so a locked
                            // reward's tooltip can explain how it's actually earned.
                            .flatMap(c => c.time_based_drops.map(drop => ({ drop, campaignDescription: c.description })))
                            .filter(({ drop }) => {
                                if (seen.has(drop.id)) return false;
                                seen.add(drop.id);
                                return true;
                            })
                            .map(({ drop, campaignDescription }) => {
                                const dp = drop.progress || progress.find(p => p.drop_id === drop.id);
                                const benefit = drop.benefit_edges?.[0];
                                const required = dp?.required_minutes_watched || drop.required_minutes_watched || 0;
                                // "Owned" per the panel's single ownership rule, so the
                                // tally always agrees with the sections and cards below.
                                const isClaimed = isDropOwned(drop, dp);
                                const current = isClaimed ? required : (dp?.current_minutes_watched || 0);
                                const percent = required > 0 ? Math.min((current / required) * 100, 100) : 0;
                                const isCollectible = isDropCollectible(drop, game.inventory_items);
                                return {
                                    dropId: drop.id,
                                    image: benefit?.image_url || '',
                                    name: benefit?.name || drop.name,
                                    requiredMinutes: required,
                                    percent,
                                    isClaimed,
                                    isReady: !isClaimed && percent >= 100,
                                    isInProgress: !isClaimed && percent > 0 && percent < 100,
                                    isCollectible,
                                    kind: getRewardKind(benefit, knownBadgeTitles),
                                    // Only locked, unearned rewards need a "how to unlock" hint.
                                    // The reward name is the strongest signal (e.g. "Gifted Sub Drop"),
                                    // with the campaign description as the fallback source.
                                    requirement: !isCollectible && !isClaimed ? unlockRequirementText(benefit?.name || drop.name, campaignDescription) : null,
                                };
                            })
                            .sort((a, b) => {
                                const am = a.requiredMinutes > 0 ? a.requiredMinutes : Number.MAX_SAFE_INTEGER;
                                const bm = b.requiredMinutes > 0 ? b.requiredMinutes : Number.MAX_SAFE_INTEGER;
                                return am - bm;
                            });

                        if (rewards.length === 0) return null;
                        const earnedCount = rewards.filter(r => r.isClaimed).length;

                        return (
                            <div className="glass-panel p-4">
                                <div className="flex items-center gap-2 mb-3">
                                    <Gift size={16} className="text-accent" />
                                    <h4 className="text-[12px] font-semibold uppercase tracking-[0.14em] text-textPrimary">Rewards</h4>
                                    <span className="text-[10px] font-mono text-textMuted bg-background/50 px-2 py-0.5 rounded ml-auto">
                                        {earnedCount}/{rewards.length} earned
                                    </span>
                                </div>
                                <div className="grid grid-cols-4 sm:grid-cols-5 gap-x-2.5 gap-y-5">
                                    {rewards.map(r => (
                                        <Tooltip
                                            key={r.dropId}
                                            content={
                                                r.requirement
                                                    ? (
                                                        <div className="text-left max-w-[15rem]">
                                                            <div className="font-semibold text-textPrimary">{r.name}</div>
                                                            <div className="mt-1 flex items-center gap-1 text-warning text-[10px] font-semibold uppercase tracking-wide">
                                                                <Ban size={9} /> How to unlock
                                                            </div>
                                                            <div className="mt-0.5 font-normal text-textSecondary leading-snug">{r.requirement}</div>
                                                        </div>
                                                    )
                                                    : `${r.name} · ${rewardKindLabel(r.kind)}${r.requiredMinutes > 0 ? ` · ${r.requiredMinutes}m` : ''}`
                                            }
                                            delay={200}
                                            side="top"
                                        >
                                            <div className="relative aspect-square">
                                                {/* Clipped art box. overflow-hidden lives here so the kind
                                                    plate below can hang off the bottom border unclipped. */}
                                                <div className="absolute inset-0 rounded-lg border border-borderLight bg-background overflow-hidden">
                                                    {r.image ? (
                                                        <img
                                                            src={r.image}
                                                            alt={r.name}
                                                            loading="lazy"
                                                            className={`w-full h-full object-contain p-1.5 ${r.isClaimed ? 'opacity-40' : ''}`}
                                                        />
                                                    ) : (
                                                        <div className="w-full h-full flex items-center justify-center">
                                                            <Gift size={20} className="text-textMuted" />
                                                        </div>
                                                    )}

                                                    {r.isClaimed && (
                                                        <div className="absolute top-1 right-1 w-4 h-4 rounded-full bg-success flex items-center justify-center border border-background">
                                                            <Check size={9} className="text-white" />
                                                        </div>
                                                    )}

                                                    {r.isReady && (
                                                        <div className="absolute top-1 right-1 w-4 h-4 rounded-full bg-warning flex items-center justify-center border border-background">
                                                            <span className="text-[8px] font-bold text-black">!</span>
                                                        </div>
                                                    )}

                                                    {!r.isCollectible && !r.isClaimed && (
                                                        <div className="absolute top-1 left-1 text-warning">
                                                            <Ban size={11} />
                                                        </div>
                                                    )}

                                                    {(r.isInProgress || r.isReady) && (
                                                        <div className="absolute inset-x-0 bottom-0 h-1 bg-background/70">
                                                            <div className="h-full bg-accent" style={{ width: `${r.percent}%` }} />
                                                        </div>
                                                    )}
                                                </div>

                                                {/* Reward kind plate, straddling the bottom border like a name tag */}
                                                <div className="absolute bottom-0 left-1/2 -translate-x-1/2 translate-y-1/2 z-10">
                                                    <span className={`px-2 py-0.5 rounded-full text-[10px] font-bold uppercase tracking-wide bg-background border border-borderLight shadow-sm ${r.kind === 'badge' ? 'text-accent' : r.kind === 'emote' ? 'text-textSecondary' : 'text-textMuted'}`}>
                                                        {rewardKindLabel(r.kind)}
                                                    </span>
                                                </div>
                                            </div>
                                        </Tooltip>
                                    ))}
                                </div>
                            </div>
                        );
                    })()}

                    {/* Currently Automation Section - Shows ONLY drops from the specific campaign being collected */}
                    {(() => {
                        // Get the current campaign being collected (from dropProgress)
                        const currentCampaignName = dropProgress?.current_campaign;
                        
                        // Get ALL drops from this game's campaigns
                        const dropsFromCampaigns = game.active_campaigns.flatMap(c => c.time_based_drops);

                        // ALSO get drops from inventory_items (which updates immediately with progress)

                        const dropsFromInventory = game.inventory_items.flatMap(item =>
                            item.campaign.time_based_drops
                        );

                        // Combine drops from both sources for lookup
                        const allDropsForGame = [...dropsFromCampaigns, ...dropsFromInventory];

                        // Build a LOCAL map: drop_id -> { drop, campaignName }
                        // Include campaign name so we can filter by specific campaign
                        const localDropMap = new Map<string, { drop: typeof dropsFromCampaigns[0]; campaignName: string }>();
                        game.active_campaigns.forEach(campaign => {
                            campaign.time_based_drops.forEach(drop => {
                                localDropMap.set(drop.id, { drop, campaignName: campaign.name });
                            });
                        });
                        game.inventory_items.forEach(item => {
                            item.campaign.time_based_drops.forEach(drop => {
                                localDropMap.set(drop.id, { drop, campaignName: item.campaign.name });
                            });
                        });

                        // Build a GLOBAL drop map from ALL games' campaigns and inventory
                        // This allows us to find metadata for drops we're automation that aren't in the current game's data
                        const globalDropMap = new Map<string, { drop: typeof allDropsForGame[0]; gameName: string; campaignName: string }>();
                        allGames.forEach(g => {
                            // From active campaigns
                            g.active_campaigns.forEach(campaign => {
                                campaign.time_based_drops.forEach(drop => {
                                    globalDropMap.set(drop.id, { drop, gameName: g.name, campaignName: campaign.name });
                                });
                            });
                            // From inventory items
                            g.inventory_items.forEach(item => {
                                item.campaign.time_based_drops.forEach(drop => {
                                    globalDropMap.set(drop.id, { drop, gameName: g.name, campaignName: item.campaign.name });
                                });
                            });
                        });



                        // DEBUG: Log all IDs for comparison
                        Logger.debug('[GameDetailPanel] Game:', game.name);
                        Logger.debug('[GameDetailPanel] Current campaign being collected:', currentCampaignName);
                        Logger.debug('[GameDetailPanel] Drops from this game:', localDropMap.size);
                        Logger.debug('[GameDetailPanel] Global drops available:', globalDropMap.size);
                        Logger.debug('[GameDetailPanel] All progress entries:', progress.length);
                        Logger.debug('[GameDetailPanel] Progress drop_ids:', progress.map(p => p.drop_id));

                        // Filter progress entries that are actively being collected:
                        // - Has some progress (current_minutes > 0)
                        // - NOT yet 100% complete (still automation)
                        // - NOT claimed
                        // Drops at 100% go to "Your Collection" section instead
                        const _totalMinutesWatched = progress.filter(p =>
                            p.current_minutes_watched > 0 &&
                            !p.is_claimed &&
                            p.current_minutes_watched < p.required_minutes_watched // Not yet 100%
                        );

                        // ONLY show progress for drops that belong to the SPECIFIC CAMPAIGN being collected
                        // This prevents showing drops from other campaigns in the same game
                        const progressForThisGame = _totalMinutesWatched.filter(p => {
                            // First check if this drop belongs to this game
                            const localLookup = localDropMap.get(p.drop_id);
                            const globalLookup = globalDropMap.get(p.drop_id);
                            
                            const belongsToThisGame = localLookup || (globalLookup && globalLookup.gameName === game.name);
                            if (!belongsToThisGame) return false;
                            
                            // If we know what campaign is being collected, filter to ONLY that campaign's drops
                            if (currentCampaignName) {
                                const dropCampaignName = localLookup?.campaignName || globalLookup?.campaignName;
                                if (dropCampaignName && dropCampaignName !== currentCampaignName) {
                                    Logger.debug(`[GameDetailPanel] Filtering out drop ${p.drop_id} - belongs to "${dropCampaignName}", automation "${currentCampaignName}"`);
                                    return false;
                                }
                            }
                            
                            return true;
                        });

                        Logger.debug('[GameDetailPanel] Active progress entries for current campaign:', progressForThisGame.length);

                        // Map each progress entry to its drop object (for benefit image/name)
                        const dropsWithProgress = progressForThisGame.map(dropProg => {
                            // First try local map (current game), then fall back to global map
                            const localLookup = localDropMap.get(dropProg.drop_id);
                            const globalLookup = globalDropMap.get(dropProg.drop_id);

                            if (localLookup) {
                                // Found in current game's data
                                const { drop: localDrop } = localLookup;
                                const benefitImage = localDrop.benefit_edges?.[0]?.image_url || '';
                                const benefitName = localDrop.benefit_edges?.[0]?.name || localDrop.name;
                                Logger.debug('[GameDetailPanel] ✓ Local match:', dropProg.drop_id, '→', benefitName, benefitImage ? '(has image)' : '(no image)');
                                return {
                                    dropId: localDrop.id,
                                    progress: dropProg,
                                    benefitImage,
                                    benefitName,
                                    gameName: game.name,
                                    hasDropObject: true,
                                };
                            } else if (globalLookup) {
                                // Found in another game's data
                                const { drop: globalDrop, gameName: dropGameName } = globalLookup;
                                const benefitImage = globalDrop.benefit_edges?.[0]?.image_url || '';
                                const benefitName = globalDrop.benefit_edges?.[0]?.name || globalDrop.name;
                                Logger.debug('[GameDetailPanel] ✓ Global match:', dropProg.drop_id, '→', benefitName, `(from ${dropGameName})`, benefitImage ? '(has image)' : '(no image)');
                                return {
                                    dropId: globalDrop.id,
                                    progress: dropProg,
                                    benefitImage,
                                    benefitName,
                                    gameName: dropGameName,
                                    hasDropObject: true,
                                };
                            } else {
                                // Progress exists but no matching drop object found anywhere
                                // Show fallback UI with just the progress data
                                Logger.debug('[GameDetailPanel] ✗ No drop match for:', dropProg.drop_id);
                                return {
                                    dropId: dropProg.drop_id,
                                    progress: dropProg,
                                    benefitImage: '',
                                    benefitName: `Drop in progress`,
                                    gameName: undefined,
                                    hasDropObject: false,
                                };
                            }
                        });

                        Logger.debug('[GameDetailPanel] Final drops with progress:', dropsWithProgress.length, 'matched:', dropsWithProgress.filter(d => d.hasDropObject).length);

                        // Only show "Currently Automation" section if we are actually automation THIS specific game
                        // This prevents showing the automation UI when viewing a different game's panel
                        if (!isProgressingThisGame) return null;

                        // Don't show section if no drops with progress for this game
                        if (dropsWithProgress.length === 0) return null;

                        return (
                            <div className="glass-panel p-4 space-y-3 border border-success/30 bg-success/5">
                                <div className="flex items-center justify-between">
                                    <div className="flex items-center gap-2 text-success text-[12px] font-semibold uppercase tracking-[0.14em]">
                                        <span className="relative flex h-2.5 w-2.5">
                                            <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-success opacity-75"></span>
                                            <span className="relative inline-flex rounded-full h-2.5 w-2.5 bg-success"></span>
                                        </span>
                                        Currently earning
                                        {dropsWithProgress.length > 0 && (
                                            <span className="text-[10px] font-mono text-success bg-success/20 px-1.5 py-0.5 rounded">
                                                {dropsWithProgress.length} drop{dropsWithProgress.length !== 1 ? 's' : ''}
                                            </span>
                                        )}
                                    </div>
                                    {/* Stop only makes sense when a provider is driving; native
                                        watch-to-earn is stopped by not watching. */}
                                    {externalDropsProvider && (
                                        <button
                                            onClick={onStopAutomation}
                                            className="glass-button px-2.5 py-1.5 text-xs font-medium text-error flex items-center gap-1.5"
                                        >
                                            <Pause size={12} />
                                            Stop
                                        </button>
                                    )}
                                </div>

                                {/* Show ALL drops with active progress */}
                                {dropsWithProgress.length > 0 ? (
                                    <div className="space-y-2">
                                        {dropsWithProgress.map(({ dropId, progress: dropProg, benefitImage, benefitName }) => {
                                            const currentMins = dropProg.current_minutes_watched;
                                            const requiredMins = dropProg.required_minutes_watched;
                                            const percent = requiredMins > 0 ? (currentMins / requiredMins) * 100 : 0;

                                            return (
                                                <div
                                                    key={dropId}
                                                    className="flex items-center gap-3 p-2 rounded-lg bg-background/50 border border-success/20"
                                                >
                                                    {/* Benefit Image */}
                                                    <div className="relative shrink-0">
                                                        {benefitImage ? (
                                                            <img
                                                                src={benefitImage}
                                                                alt={benefitName}
                                                                className="w-12 h-12 rounded-lg object-contain border border-success/30 bg-background"
                                                            />
                                                        ) : (
                                                            <div className="w-12 h-12 rounded-lg bg-background border border-success/30 flex items-center justify-center">
                                                                <Gift size={18} className="text-success" />
                                                            </div>
                                                        )}
                                                    </div>

                                                    {/* Drop Info */}
                                                    <div className="flex-1 min-w-0">
                                                        <Tooltip content={benefitName} delay={300} side="top">
                                                            <span className="text-xs font-medium text-textPrimary truncate block">
                                                                {benefitName}
                                                            </span>
                                                        </Tooltip>
                                                        <div className="h-2 w-full bg-background rounded-full mt-1.5 overflow-hidden border border-borderSubtle">
                                                            <div
                                                                className="h-full rounded-full animate-progress-shimmer"
                                                                style={{ width: `${Math.min(percent, 100)}%` }}
                                                            />
                                                        </div>
                                                        <div className="flex items-center justify-between mt-1">
                                                            <span className="text-[10px] text-success font-mono">
                                                                {Math.round(currentMins)}/{requiredMins}m
                                                            </span>
                                                            <span className="text-[10px] text-textMuted font-semibold">
                                                                {Math.round(percent)}%
                                                            </span>
                                                        </div>
                                                    </div>
                                                </div>
                                            );
                                        })}
                                    </div>
                                ) : (
                                    // Fallback when automation but no WebSocket progress yet
                                    <div className="flex items-center gap-3 p-2 bg-background/50 rounded-lg border border-success/20">
                                        {collectDropImage && (
                                            <img
                                                src={collectDropImage}
                                                alt={collectBenefitName}
                                                className="w-12 h-12 rounded-lg object-contain border border-success/40 bg-background shrink-0"
                                            />
                                        )}
                                        <div className="flex-1 min-w-0">
                                            <Tooltip content={collectBenefitName || collectDropName} delay={300} side="top">
                                                <span className="text-sm font-medium text-textPrimary truncate block">
                                                    {collectBenefitName || collectDropName || 'Starting...'}
                                                </span>
                                            </Tooltip>
                                            <div className="h-2 w-full bg-background rounded-full mt-1.5 overflow-hidden">
                                                <div
                                                    className="h-full rounded-full animate-progress-shimmer"
                                                    style={{ width: `${Math.min(collectProgress, 100)}%` }}
                                                />
                                            </div>
                                            <p className="text-xs text-success font-mono mt-1">
                                                Waiting for progress update...
                                            </p>
                                        </div>
                                    </div>
                                )}
                            </div>
                        );
                    })()}

                    {/* Active Campaigns Section */}
                    {(()=> {
                        // Merge progress from inventory into campaigns for accurate status checking
                        const mergedCampaigns = campaignsWithMergedProgress;

                        // A campaign is Active while ANY of its watch-time rewards
                        // is still earnable under the panel's single ownership rule.
                        const incompleteCampaigns = mergedCampaigns.filter(campaign =>
                            campaign.time_based_drops.some(isDropEarnable)
                        );

                        if (incompleteCampaigns.length === 0) return null;

                        return (
                            <div>
                                <h4 className="flex items-center gap-2 text-[12px] font-semibold uppercase tracking-[0.14em] text-textPrimary mb-3">
                                    <Gift size={16} className="text-accent" />
                                    Active Campaigns
                                    <span className="text-[10px] font-mono text-textMuted bg-background/50 px-2 py-0.5 rounded ml-auto">
                                        {incompleteCampaigns.length} remaining
                                    </span>
                                </h4>

                                <div className="space-y-4">
                                    {incompleteCampaigns.map(campaign => (
                                        <CampaignCard
                                            key={campaign.id}
                                            campaign={campaign}
                                            inventoryItems={game.inventory_items}
                                            progress={progress}
                                            isDropOwned={isDropOwned}
                                            dropProgress={dropProgress}
                                            onClaimDrop={onClaimDrop}
                                            onWatch={() => openWatchPicker(campaign)}
                                            pickChannel={() => requestPickChannel(campaign)}
                                        />
                                    ))}
                                </div>
                            </div>
                        );
                    })()}

                    {/* Completed Campaigns Section */}
                    {(() => {
                        // Use merged campaigns for accurate status
                        const mergedCampaigns = campaignsWithMergedProgress;

                        // A campaign is Completed when it's the exact complement of
                        // Active: it has at least one watch-time reward, and NONE of
                        // its watch-time rewards are still earnable (all owned, claimed,
                        // or 100% watched). Using the same isDropEarnable rule as Active
                        // is what stops a fully-earned campaign from vanishing — the old
                        // check demanded per-drop progress data a reissued/earned
                        // campaign never carries, so it showed in neither section.
                        const completedCampaigns = mergedCampaigns.filter(campaign => {
                            const hasWatchTimeReward = campaign.time_based_drops.some(drop => {
                                const dp = drop.progress || progress.find(p => p.drop_id === drop.id);
                                return (dp?.required_minutes_watched || drop.required_minutes_watched || 0) > 0;
                            });
                            if (!hasWatchTimeReward) return false;
                            return !campaign.time_based_drops.some(isDropEarnable);
                        });

                        if (completedCampaigns.length === 0) return null;

                        return (
                            <div>
                                <h4 className="flex items-center gap-2 text-[12px] font-semibold uppercase tracking-[0.14em] text-textPrimary mb-3">
                                    <Check size={16} className="text-success" />
                                    Completed Campaigns
                                    <span className="text-[10px] font-mono text-success bg-success/10 px-2 py-0.5 rounded ml-auto">
                                        {completedCampaigns.length} done
                                    </span>
                                </h4>

                                <div className="space-y-4">
                                    {completedCampaigns.map(campaign => (
                                        <CampaignCard
                                            key={campaign.id}
                                            campaign={campaign}
                                            inventoryItems={game.inventory_items}
                                            progress={progress}
                                            isDropOwned={isDropOwned}
                                            dropProgress={dropProgress}
                                            onClaimDrop={onClaimDrop}
                                            onWatch={() => openWatchPicker(campaign)}
                                            pickChannel={() => requestPickChannel(campaign)}
                                        />
                                    ))}
                                </div>
                            </div>
                        );
                    })()}

                    {/* No Active Campaigns */}
                    {game.active_campaigns.length === 0 && (
                        <div className="glass-panel p-6 text-center border border-dashed border-borderLight">
                            <Gift size={32} className="mx-auto text-textSecondary opacity-40 mb-2" />
                            <p className="text-sm text-textSecondary">
                                No active campaigns for this game right now.
                            </p>
                        </div>
                    )}

                    {/* Your Collection - Shows ONLY completed drops (100% watched) from inventory */}
                    {(() => {
                        // Find completed drops (100% watched) - NOT in-progress
                        const localCompletedDrops: Array<{
                            dropId: string;
                            dropInstanceId?: string;
                            benefitImage: string;
                            benefitName: string;
                            isClaimed: boolean;
                            isCollectible: boolean; // Track if this drop can be collected (time-based)
                        }> = [];

                        // Track which drops we've added to avoid duplicates
                        const addedDropIds = new Set<string>();

                        // DEBUG: Log what we're receiving


                        // 1. Check inventory_items for completed/claimed drops
                        // Each inventory item has its own progress data
                        game.inventory_items.forEach(item => {


                            item.campaign.time_based_drops.forEach((drop, dropIndex) => {
                                // Check if this drop has internal progress data showing it's complete
                                const dropProgress = drop.progress;

                                // DEBUG: Log each drop's progress
                                Logger.debug(`[Your Collection] Drop ${dropIndex}:`, drop.id, drop.name);
                                Logger.debug('  - progress:', dropProgress);
                                Logger.debug('  - drop.required_minutes_watched:', drop.required_minutes_watched);

                                // Check completion using multiple methods:
                                // 1. Progress field shows 100%
                                const isCompleteByProgress = dropProgress &&
                                    dropProgress.current_minutes_watched >= dropProgress.required_minutes_watched &&
                                    dropProgress.required_minutes_watched > 0;

                                // 2. Progress field shows 100% using drop's required minutes (fallback)
                                const isCompleteByDropMinutes = dropProgress &&
                                    dropProgress.current_minutes_watched >= drop.required_minutes_watched &&
                                    drop.required_minutes_watched > 0;

                                const isComplete = isCompleteByProgress || isCompleteByDropMinutes;
                                const isClaimed = dropProgress?.is_claimed || false;

                                // Also check if claimed based on claimed_drops count
                                const isClaimedByIndex = dropIndex < item.claimed_drops;

                                Logger.debug('  - isComplete:', isComplete, 'isClaimed:', isClaimed, 'isClaimedByIndex:', isClaimedByIndex);

                                // Include if: (a) complete based on progress, or (b) claimed based on index
                                if (isComplete || (dropProgress?.is_claimed === true)) {
                                    if (!addedDropIds.has(drop.id)) {
                                        addedDropIds.add(drop.id);
                                        localCompletedDrops.push({
                                            dropId: drop.id,
                                            dropInstanceId: dropProgress?.drop_instance_id,
                                            benefitImage: drop.benefit_edges?.[0]?.image_url || '',
                                            benefitName: drop.benefit_edges?.[0]?.name || drop.name,
                                            isClaimed: isClaimed || isClaimedByIndex,
                                            isCollectible: isDropCollectible(drop, game.inventory_items),
                                        });
                                        Logger.debug('  ✓ Added to collection, drop_instance_id:', dropProgress?.drop_instance_id);
                                    }
                                }
                            });
                        });

                        // 2. Check progress array for any additional completed drops
                        // (in case progress data is more up-to-date than inventory)
                        // Build a set of valid drop IDs for this game
                        const gameDropIds = new Set<string>();
                        const dropInfoMap = new Map<string, { benefitImage: string; benefitName: string }>();

                        game.inventory_items.forEach(item => {
                            item.campaign.time_based_drops.forEach(drop => {
                                gameDropIds.add(drop.id);
                                dropInfoMap.set(drop.id, {
                                    benefitImage: drop.benefit_edges?.[0]?.image_url || '',
                                    benefitName: drop.benefit_edges?.[0]?.name || drop.name,
                                });
                            });
                        });

                        // 3. ALSO check active campaigns for completed drops (100% or claimed in current session)
                        game.active_campaigns.forEach(campaign => {
                            campaign.time_based_drops.forEach(drop => {
                                gameDropIds.add(drop.id);
                                if (!dropInfoMap.has(drop.id)) {
                                    dropInfoMap.set(drop.id, {
                                        benefitImage: drop.benefit_edges?.[0]?.image_url || '',
                                        benefitName: drop.benefit_edges?.[0]?.name || drop.name,
                                    });
                                }
                                
                                // If this drop is complete or claimed, add it to the collection
                                if (!addedDropIds.has(drop.id)) {
                                    const dropProgress = drop.progress || progress.find(p => p.drop_id === drop.id);
                                    if (dropProgress) {
                                        const isComplete = dropProgress.current_minutes_watched >= dropProgress.required_minutes_watched &&
                                            dropProgress.required_minutes_watched > 0;
                                        const isClaimed = dropProgress.is_claimed;
                                        
                                        if (isComplete || isClaimed) {
                                            addedDropIds.add(drop.id);
                                            localCompletedDrops.push({
                                                dropId: drop.id,
                                                dropInstanceId: dropProgress.drop_instance_id,
                                                benefitImage: drop.benefit_edges?.[0]?.image_url || '',
                                                benefitName: drop.benefit_edges?.[0]?.name || drop.name,
                                                isClaimed,
                                                isCollectible: isDropCollectible(drop, game.inventory_items),
                                            });

                                        }
                                    }
                                }
                            });
                        });

                        // 4. ALSO check for drops completed via benefit ID matching (from backend completedDrops)
                        // These are drops that are "owned" but have no progress data, so they didn't appear in previous checks
                        // Create a completed benefit IDs set from backend data
                        const completedBenefitIds = ownedBenefitIds;
                        
                        // Scan all campaigns for drops with matching benefit IDs
                        const allCampaigns = [...game.active_campaigns, ...game.inventory_items.map(item => item.campaign)];

                        
                        allCampaigns.forEach(campaign => {

                            campaign.time_based_drops.forEach(drop => {
                                // Skip if already added
                                if (addedDropIds.has(drop.id)) {

                                    return;
                                }
                                

                                
                                // Check if this drop's benefit ID matches a completed benefit
                                const hasBenefitMatch = drop.benefit_edges?.some(
                                    benefit => completedBenefitIds.has(benefit.id)
                                );
                                
                                // ALSO treat a badge drop as owned when its benefit name
                                // matches an earned badge title (distribution_type is null in
                                // the frontend data, so we can't rely on it).
                                const hasBadgeNameMatch = matchesEarnedBadge(drop.benefit_edges?.[0]?.name, earnedBadgeTitles);

                                if (hasBenefitMatch || hasBadgeNameMatch) {
                                    // This drop is owned via benefit ID (e.g., badge drop or item drop from expired campaign)
                                    addedDropIds.add(drop.id);
                                    localCompletedDrops.push({
                                        dropId: drop.id,
                                        benefitImage: drop.benefit_edges?.[0]?.image_url || '',
                                        benefitName: drop.benefit_edges?.[0]?.name || drop.name,
                                        isClaimed: true, // Treat benefit-matched drops as claimed
                                        isCollectible: false, // No progress data means not currently collectible
                                    });

                                }
                            });
                        });

                        progress.forEach(p => {
                            if (!gameDropIds.has(p.drop_id)) return;
                            if (addedDropIds.has(p.drop_id)) return; // Skip if already added

                            // Include if:
                            // - 100% complete (ready-to-claim)
                            // - OR already claimed
                            // Some claimed drops don't always keep an intuitive minutes state, so
                            // we treat `is_claimed` as authoritative for "completed".
                            const isComplete = p.current_minutes_watched >= p.required_minutes_watched &&
                                p.required_minutes_watched > 0; // Only if collectible (has required watch time)
                            if (isComplete || p.is_claimed) {
                                const dropInfo = dropInfoMap.get(p.drop_id);
                                if (dropInfo) {
                                    addedDropIds.add(p.drop_id);
                                    localCompletedDrops.push({
                                        dropId: p.drop_id,
                                        benefitImage: dropInfo.benefitImage,
                                        benefitName: dropInfo.benefitName,
                                        isClaimed: p.is_claimed,
                                        isCollectible: p.required_minutes_watched > 0, // Collectible if has required watch time
                                    });
                                }
                            }
                        });

                        // Sort: unclaimed first, then claimed
                        const sortedDrops = localCompletedDrops.sort((a, b) => {
                            if (a.isClaimed === b.isClaimed) return 0;
                            return a.isClaimed ? 1 : -1; // Unclaimed first
                        });

                        // Separate into unclaimed (ready to claim) and claimed
                        const unclaimedDrops = sortedDrops.filter(d => !d.isClaimed);
                        const claimedDrops = sortedDrops.filter(d => d.isClaimed);

                        const totalItems = sortedDrops.length;
                        if (totalItems === 0) return null;

                        return (
                            <div>
                                <h4 className="flex items-center gap-2 text-[12px] font-semibold uppercase tracking-[0.14em] text-textPrimary mb-3">
                                    <Package size={16} className="text-highlight-purple" />
                                    Your Collection
                                    <span className="text-[10px] font-mono text-highlight-purple bg-highlight-purple/10 px-2 py-0.5 rounded ml-auto">
                                        {totalItems} item{totalItems !== 1 ? 's' : ''}
                                    </span>
                                </h4>

                                {/* Unclaimed completed drops - shown first with Claim button */}
                                {unclaimedDrops.length > 0 && (
                                    <div className="space-y-2 mb-4">
                                        {unclaimedDrops.map(({ dropId, dropInstanceId, benefitImage, benefitName }) => (
                                            <div
                                                key={dropId}
                                                className="flex items-center gap-3 p-2 rounded-lg bg-warning/10 border border-warning/30"
                                            >
                                                {/* Benefit Image */}
                                                <div className="relative shrink-0">
                                                    {benefitImage ? (
                                                        <img
                                                            src={benefitImage}
                                                            alt={benefitName}
                                                            className="w-12 h-12 rounded-lg object-contain border border-warning/30 bg-background"
                                                        />
                                                    ) : (
                                                        <div className="w-12 h-12 rounded-lg bg-background border border-warning/30 flex items-center justify-center">
                                                            <Gift size={18} className="text-warning" />
                                                        </div>
                                                    )}
                                                    {/* Ready badge */}
                                                    <div className="absolute -top-1 -right-1 w-5 h-5 rounded-full bg-warning flex items-center justify-center shadow-lg animate-pulse">
                                                        <span className="text-[8px] font-bold text-black">!</span>
                                                    </div>
                                                </div>

                                                {/* Drop Info */}
                                                <div className="flex-1 min-w-0">
                                                    <Tooltip content={benefitName} delay={300} side="top">
                                                        <span className="text-xs font-medium text-textPrimary truncate block">
                                                            {benefitName}
                                                        </span>
                                                    </Tooltip>
                                                    <p className="text-[10px] text-warning font-semibold mt-0.5">
                                                        Ready to claim!
                                                    </p>
                                                </div>

                                                {/* Claim Button */}
                                                <button
                                                    onClick={() => onClaimDrop(dropId, dropInstanceId)}
                                                    className="px-3 py-1.5 bg-success hover:bg-success/80 text-white text-xs font-bold rounded-lg transition-all shadow-lg animate-pulse shrink-0"
                                                >
                                                    Claim
                                                </button>
                                            </div>
                                        ))}
                                    </div>
                                )}

                                {/* Grid of claimed drops */}
                                {claimedDrops.length > 0 && (
                                    <div className="grid grid-cols-4 gap-4">
                                        {claimedDrops.map(({ dropId, benefitImage, benefitName }) => (
                                            <Tooltip key={dropId} content={benefitName} delay={200} side="top">
                                            <div
                                                className="group relative pt-1 pr-1"
                                            >
                                                {/* Drop Reward Image Container */}
                                                <div className="w-full aspect-square rounded-lg border border-highlight-purple/40 bg-highlight-purple/10 p-1">
                                                    {benefitImage ? (
                                                        <img
                                                            src={benefitImage}
                                                            alt={benefitName}
                                                            className="w-full h-full object-contain rounded-md"
                                                            loading="lazy"
                                                        />
                                                    ) : (
                                                        <div className="w-full h-full flex items-center justify-center rounded-md bg-background/50">
                                                            <Gift size={20} className="text-highlight-purple" />
                                                        </div>
                                                    )}
                                                </div>

                                                {/* Checkmark badge - positioned outside the container */}
                                                <div className="absolute top-0 right-0 w-5 h-5 rounded-full bg-success flex items-center justify-center shadow-lg border-2 border-background">
                                                    <Check size={10} className="text-white" />
                                                </div>

                                                {/* Name on hover */}
                                                <div className="absolute -bottom-5 left-0 right-0 text-center opacity-0 group-hover:opacity-100 transition-opacity z-10">
                                                    <span className="text-[9px] text-textMuted bg-background/90 px-1.5 py-0.5 rounded truncate max-w-full inline-block">
                                                        {benefitName}
                                                    </span>
                                                </div>
                                            </div>
                                            </Tooltip>
                                        ))}
                                    </div>
                                )}
                            </div>
                        );
                    })()}
                </div>
            </div>
        </div>

        {picker && (
            <ChannelPickerModal
                isOpen
                onClose={closePicker}
                campaignId={picker.campaign.id}
                campaignName={picker.campaign.name}
                gameName={picker.campaign.game_name}
                gameId={picker.campaign.game_id}
                allowedChannels={picker.campaign.allowed_channels}
                isAclBased={picker.campaign.is_acl_based}
                actionLabel={picker.actionLabel}
                onPick={picker.onPick}
            />
        )}
        </>
    );
}

// Sub-component for campaign cards
interface CampaignCardProps {
    campaign: DropCampaign;
    inventoryItems: InventoryItem[];
    progress: DropProgress[];
    // The panel's single ownership rule. The card must judge each reward with
    // the SAME predicate its section was filtered by, or a "Completed"
    // campaign can render an unearned-looking card that still offers Mine.
    isDropOwned: (drop: TimeBasedDrop, dp?: DropProgress | null) => boolean;
    dropProgress: DropProgressStatus | null;
    onClaimDrop: (dropId: string, dropInstanceId?: string) => void;
    onWatch: () => void; // native: open the channel picker to watch this campaign
    pickChannel: () => Promise<PickedDropChannel | null>; // plugin: open the picker, resolve the choice
}

function CampaignCard({
    campaign,
    inventoryItems,
    progress,
    isDropOwned,
    dropProgress,
    onClaimDrop,
    onWatch,
    pickChannel,
}: CampaignCardProps) {
    // Per-campaign controls are contributed by a provider (an opt-in plugin)
    // into a generic slot; core renders whatever is hung there and passes the
    // campaign context. With no provider this is empty and the card just shows
    // native progress.
    const cardActions = usePluginUiRegistry(selectSlot(DROPS_CARD_ACTION_SLOT));
    // Prefer embedded progress (drop.progress) and fall back to the global progress[] state.
    const resolveDropProgress = (dropId: string, embedded?: DropProgress) => {
        return progress.find(p => p.drop_id === dropId) || embedded || null;
    };
    
    // Check if this campaign is collectible (has time-based drops with watch time requirements)
    // Pass inventory items to check them as a fallback source for required_minutes_watched
    const isCollectible = isCampaignCollectible(campaign, inventoryItems);
    const dropType = getCampaignDropType(campaign, inventoryItems);

    // Get all drop rewards with their images - directly from drops
    const dropRewards = campaign.time_based_drops.map(drop => {
        const benefit = drop.benefit_edges?.[0];
        const dropProgress = resolveDropProgress(drop.id, drop.progress);

        // Owned per the panel's shared rule (own claim, claimed inventory drop,
        // same benefit id, or a held badge). Watch-time in progress wins inside
        // the rule, so a fresh drop is never hidden by a cross-campaign match.
        const isGloballyCompleted = isDropOwned(drop, dropProgress);

        const required = dropProgress?.required_minutes_watched || drop.required_minutes_watched || 0;
        const current = dropProgress
            ? (dropProgress.is_claimed ? required : (dropProgress.current_minutes_watched || 0))
            : (isGloballyCompleted ? required : 0);

        const progressPercent = required > 0 ? (current / required) * 100 : 0;

        return {
            dropId: drop.id,
            dropName: drop.name,
            requiredMinutes: drop.required_minutes_watched,
            imageUrl: benefit?.image_url || '',
            benefitName: benefit?.name || drop.name,
            isClaimed: dropProgress?.is_claimed || false, // Only trust actual claim status, not benefit ID matching
            // Earned = claimed here OR owned from a previous/expired campaign instance.
            // Drives the "done" look in the timeline so a completed campaign doesn't
            // render as grey/0%. isClaimed stays for the actual Claim affordance.
            isEarned: (dropProgress?.is_claimed || false) || isGloballyCompleted,
            progressPercent, // Show actual progress, not forced 100%
            isInProgress: progressPercent > 0 && progressPercent < 100 && !dropProgress?.is_claimed,
            isCollectible: isDropCollectible(drop, inventoryItems), // Track if drop is collectible - check inventory as fallback
            isGloballyCompleted, // Track if this drop was earned from a previous/expired campaign
        };
    });

    // Every watch-time reward already earned — the campaign is done, so we show a
    // "Completed" tag instead of prompting the user to go watch it again.
    const watchTimeRewards = dropRewards.filter(r => (r.requiredMinutes || 0) > 0);
    const allEarned = watchTimeRewards.length > 0 && watchTimeRewards.every(r => r.isEarned);

    // Something is still left to earn here (unclaimed, below 100%), so an
    // "Earning" badge is meaningful. A campaign whose rewards are all earned or
    // all sitting at 100% ready-to-claim isn't collecting anything. A freshly
    // started mine at 0 minutes MUST count: requiring credited minutes here
    // kept the badge hidden for the first minute(s) after clicking Mine.
    const stillCollecting = dropRewards.some(
        r => r.isCollectible && !r.isEarned && r.progressPercent < 100
    );

    // The live status reports the campaign by ID (external provider) or by
    // name (native path), so accept either.
    const isProgressingThisCampaign = !!dropProgress?.active &&
        (dropProgress.current_campaign === campaign.id ||
            dropProgress.current_campaign === campaign.name) &&
        stillCollecting;

    return (
        <div className="glass-panel p-4 border border-borderLight">
            {/* Campaign Header */}
            <div className="flex items-center justify-between mb-3">
                <div className="flex items-center gap-2 flex-1 min-w-0 pr-2">
                    <span className="text-xs text-textSecondary font-medium truncate">
                        {campaign.name}
                    </span>
                    {/* Drop Type Badge */}
                    {dropType.type === 'time' && (
                        <span className="px-1.5 py-0.5 text-[9px] font-semibold rounded bg-accent/20 text-accent border border-accent/30 shrink-0 flex items-center gap-1">
                            <Clock size={9} />
                            Watch
                        </span>
                    )}
                    {dropType.type === 'mixed' && (
                        <span className="px-1.5 py-0.5 text-[9px] font-semibold rounded bg-highlight-orange/20 text-highlight-orange border border-highlight-orange/30 shrink-0 flex items-center gap-1">
                            <Clock size={9} />
                            Mixed
                        </span>
                    )}
                    {dropType.type === 'instant' && (
                        <span className="px-1.5 py-0.5 text-[9px] font-semibold rounded bg-warning/20 text-warning border border-warning/30 shrink-0 flex items-center gap-1">
                            <Ban size={9} />
                            Event Only
                        </span>
                    )}
                    {dropType.type === 'other' && (
                        <span className="px-1.5 py-0.5 text-[9px] font-semibold rounded bg-highlight-purple/20 text-highlight-purple border border-highlight-purple/30 shrink-0 flex items-center gap-1">
                            <Star size={9} />
                            Special
                        </span>
                    )}
                </div>
                {/* Per-campaign control. By default core's way to earn a drop is
                    to go watch the category, so we show a "Watch" link that opens
                    it. A provider (opt-in plugin) can hang its own control in the
                    drops.card-action slot to take this over (e.g. earn without
                    watching), in which case we render that instead. */}
                {!isProgressingThisCampaign && allEarned && (
                    <span className="text-[10px] font-semibold text-success flex items-center gap-1 bg-success/10 px-2 py-1 rounded shrink-0">
                        <Check size={11} />
                        Completed
                    </span>
                )}
                {!isProgressingThisCampaign && isCollectible && !allEarned && (
                    cardActions.length > 0
                        ? cardActions.map((c) => {
                            const Control = c.Component as React.ComponentType<DropCardActionContext>;
                            return (
                                <Control
                                    key={`${c.pluginId}:${c.id}`}
                                    campaignId={campaign.id}
                                    campaignName={campaign.name}
                                    gameName={campaign.game_name}
                                    earnable={isCollectible}
                                    progressing={false}
                                    isAclBased={campaign.is_acl_based}
                                    allowedChannels={campaign.allowed_channels}
                                    pickChannel={pickChannel}
                                />
                            );
                        })
                        : (
                            <Tooltip content={campaign.is_acl_based ? 'Pick a participating channel to watch' : `Watch ${campaign.game_name} to earn this drop`} side="top">
                                <button
                                    onClick={(e) => {
                                        e.stopPropagation();
                                        onWatch();
                                    }}
                                    className="glass-button px-3 py-1.5 text-xs font-semibold text-accent flex items-center gap-1.5"
                                >
                                    <Tv size={12} />
                                    Watch
                                </button>
                            </Tooltip>
                        )
                )}
                {isProgressingThisCampaign && (
                    <span className="text-xs text-success font-semibold flex items-center gap-1.5 bg-success/10 px-2 py-1 rounded">
                        <span className="relative flex h-2 w-2">
                            <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-success opacity-75"></span>
                            <span className="relative inline-flex rounded-full h-2 w-2 bg-success"></span>
                        </span>
                        Earning
                    </span>
                )}
            </div>

            {/* Reward checkpoint timeline. Drops in a campaign share one cumulative
                watch-time counter, so this is a single fill bar with a marker per
                reward milestone, ordered low to high. */}
            {(() => {
                const timed = dropRewards
                    .filter(r => (r.requiredMinutes || 0) > 0)
                    .slice()
                    .sort((a, b) => a.requiredMinutes - b.requiredMinutes);

                if (timed.length === 0) return null;

                const maxRequired = timed[timed.length - 1].requiredMinutes;
                const currentMinutes = Math.min(
                    timed.reduce((mx, r) => {
                        const dp = resolveDropProgress(r.dropId);
                        const cur = r.isEarned ? r.requiredMinutes : (dp ? Math.round(dp.current_minutes_watched) : 0);
                        return Math.max(mx, cur);
                    }, 0),
                    maxRequired
                );
                const fillPercent = maxRequired > 0 ? Math.min((currentMinutes / maxRequired) * 100, 100) : 0;
                const nextReward = timed.find(r => !r.isEarned && currentMinutes < r.requiredMinutes);
                const ready = dropRewards.filter(r => !r.isEarned && r.progressPercent >= 100);

                return (
                    <div>
                        <div className="flex items-center justify-between mb-2.5 text-[10px]">
                            <span className="text-textMuted font-mono">{currentMinutes}/{maxRequired}m</span>
                            <span className="text-textMuted truncate pl-2">
                                {nextReward
                                    ? <>next: <span className="text-textSecondary">{nextReward.benefitName}</span> in {nextReward.requiredMinutes - currentMinutes}m</>
                                    : `All ${timed.length} reward${timed.length !== 1 ? 's' : ''} reached`}
                            </span>
                        </div>

                        {/* Single fill bar with a checkpoint marker per reward milestone */}
                        <div className="relative h-3">
                            <div className="absolute inset-x-0 top-1/2 -translate-y-1/2 h-1.5 rounded-full bg-background border border-borderLight overflow-hidden">
                                <div
                                    className={`h-full rounded-full ${isProgressingThisCampaign ? 'bg-accent animate-progress-shimmer' : 'bg-accent'}`}
                                    style={{ width: `${fillPercent}%` }}
                                />
                            </div>
                            {timed.map(r => {
                                const pos = maxRequired > 0 ? (r.requiredMinutes / maxRequired) * 100 : 0;
                                const reached = r.isEarned || currentMinutes >= r.requiredMinutes;
                                return (
                                    <Tooltip
                                        key={r.dropId}
                                        content={`${r.benefitName} · ${r.requiredMinutes}m${r.isClaimed ? ' · claimed' : r.isGloballyCompleted ? ' · earned' : reached ? ' · ready' : ''}`}
                                        delay={150}
                                        side="top"
                                    >
                                        <div
                                            className="absolute top-1/2 -translate-y-1/2 -translate-x-1/2"
                                            style={{ left: `${pos}%` }}
                                        >
                                            <div className={`w-3 h-3 rounded-full border-2 flex items-center justify-center transition-colors ${
                                                r.isEarned
                                                    ? 'bg-success border-success'
                                                    : reached
                                                        ? 'bg-warning border-warning'
                                                        : 'bg-background border-borderLight'
                                            }`}>
                                                {r.isEarned && <Check size={7} className="text-white" />}
                                            </div>
                                        </div>
                                    </Tooltip>
                                );
                            })}
                        </div>

                        {ready.length > 0 && (
                            <div className="mt-3 flex flex-wrap gap-1.5">
                                {ready.map(r => (
                                    <button
                                        key={r.dropId}
                                        onClick={(e) => {
                                            e.stopPropagation();
                                            const dp = resolveDropProgress(r.dropId);
                                            onClaimDrop(r.dropId, dp?.drop_instance_id);
                                        }}
                                        className="px-2.5 py-1 bg-success hover:bg-success/80 text-white text-[10px] font-bold rounded transition-all max-w-[160px] truncate"
                                    >
                                        Claim {r.benefitName}
                                    </button>
                                ))}
                            </div>
                        )}
                    </div>
                );
            })()}
        </div>
    );
}
