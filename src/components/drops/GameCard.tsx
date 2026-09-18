import { useRef, useEffect, useState } from 'react';
import { Check, Clock, Package, Square, Heart, DollarSign } from 'lucide-react';
import type { UnifiedGame, DropProgress, DropProgressStatus } from '../../types';
import { Tooltip } from '../ui/Tooltip';
import { deriveDropProgressDisplay } from '../../utils/dropProgressDisplay';
import { useAppStore } from '../../stores/AppStore';

interface GameCardProps {
    game: UnifiedGame;
    allGames: UnifiedGame[]; // All games for global drop metadata lookup
    progress: DropProgress[];
    dropProgress: DropProgressStatus | null;
    isSelected: boolean;
    isFavorite: boolean;
    onClick: () => void;
    onStopAutomation?: () => void;
    onToggleFavorite?: (gameName: string) => void;
}

// Helper to format time remaining
function formatTimeRemaining(minutes: number): string {
    if (minutes <= 0) return '0m';
    const hours = Math.floor(minutes / 60);
    const mins = Math.round(minutes % 60);
    if (hours > 0) {
        return mins > 0 ? `${hours}h ${mins}m` : `${hours}h`;
    }
    return `${mins}m`;
}

export default function GameCard({
    game,
    allGames,
    progress,
    dropProgress,
    isSelected,
    isFavorite,
    onClick,
    onStopAutomation,
    onToggleFavorite
}: GameCardProps) {
    // Stop only applies when a provider is driving; native watch-to-earn stops
    // by not watching.
    const externalDropsProvider = useAppStore((s) => s.externalDropsProvider);
    const titleRef = useRef<HTMLHeadingElement>(null);
    const containerRef = useRef<HTMLDivElement>(null);
    const [fontSize, setFontSize] = useState(14); // Start with default size in px

    // Check if this game is currently being collected
    // Use case-insensitive comparison and also check the game's active flag
    const isDropProgressing = dropProgress?.active && (
        game.active ||
        dropProgress.current_drop?.game_name?.toLowerCase() === game.name?.toLowerCase() ||
        dropProgress.current_channel?.game_name?.toLowerCase() === game.name?.toLowerCase()
    );

    // Build a global drop map from ALL games for metadata lookup
    const globalDropMap = new Map<string, { benefitImage: string; benefitName: string; gameName: string }>();
    allGames.forEach(g => {
        g.active_campaigns.forEach(campaign => {
            campaign.time_based_drops.forEach(drop => {
                globalDropMap.set(drop.id, {
                    benefitImage: drop.benefit_edges?.[0]?.image_url || '',
                    benefitName: drop.benefit_edges?.[0]?.name || drop.name,
                    gameName: g.name
                });
            });
        });
        g.inventory_items.forEach(item => {
            item.campaign.time_based_drops.forEach(drop => {
                globalDropMap.set(drop.id, {
                    benefitImage: drop.benefit_edges?.[0]?.image_url || '',
                    benefitName: drop.benefit_edges?.[0]?.name || drop.name,
                    gameName: g.name
                });
            });
        });
    });

    // Derive the automation drop to display through the shared rule so this card,
    // the title-bar badge, and the detail panel never disagree: the backend
    // picks WHICH drop (current_drop), but the minutes always come from the
    // freshest live progress[] value rather than current_drop's slower-moving
    // copy. Image/name still resolve through the global drop map when the live
    // data doesn't carry them (live value -> campaign metadata -> generic).
    const bestDrop = (() => {
        if (!isDropProgressing) return null;
        const display = deriveDropProgressDisplay(dropProgress, progress);
        if (!display) return null;
        const metadata = globalDropMap.get(display.dropId);
        return {
            dropId: display.dropId,
            current: display.currentMinutes,
            required: display.requiredMinutes,
            benefitImage: display.dropImage || metadata?.benefitImage || '',
            benefitName: display.dropName || metadata?.benefitName || 'Drop',
        };
    })();

    // Automation progress from best drop
    const collectProgress = isDropProgressing && bestDrop
        ? {
            current: bestDrop.current,
            required: bestDrop.required,
        }
        : null;

    // Get the benefit image/name from best drop
    const collectDropBenefitImage = bestDrop?.benefitImage;
    const collectDropBenefitName = bestDrop?.benefitName || dropProgress?.current_drop?.drop_name || 'Drop';

    // Transform box art URL to high resolution (1200x1600)
    // GQL API returns URLs with fixed dimensions (e.g., "52x72"), not placeholders
    // Helix API returns URLs with {width}x{height} placeholders
    // We need to handle both cases
    const getHighResBoxArt = (url: string | undefined): string => {
        if (!url) return 'https://static-cdn.jtvnw.net/ttv-static/404_boxart-1200x1600.jpg';

        // If URL has placeholders (Helix style), replace them
        if (url.includes('{width}') && url.includes('{height}')) {
            return url.replace('{width}', '1200').replace('{height}', '1600');
        }

        // If URL has fixed dimensions (GQL style), replace them with high res
        // Pattern: -WIDTHxHEIGHT.jpg or -WIDTHxHEIGHT.png
        const fixedDimensionPattern = /-\d+x\d+\.(jpg|png|jpeg|webp)/i;
        if (fixedDimensionPattern.test(url)) {
            return url.replace(fixedDimensionPattern, '-1200x1600.$1');
        }

        // Fallback: return as-is
        return url;
    };

    const boxArtUrl = getHighResBoxArt(game.box_art_url);

    // Calculate claimable drops count and overall progress
    // IMPORTANT: Prefer per-drop embedded progress (drop.progress) because it comes directly
    // from the CampaignDetails GQL `timeBasedDrops.self` and is accurate even when the
    // separate `progress[]` array hasn't been populated yet.
    let claimableCount = 0;
    let inProgressCount = 0;
    let totalMinutesWatched = 0;
    let totalMinutesRequired = 0;

    const getProgressForDrop = (dropId: string, embedded?: DropProgress) => {
        return progress.find(p => p.drop_id === dropId) || embedded || null;
    };

    game.active_campaigns.forEach(campaign => {
        campaign.time_based_drops.forEach(drop => {
            const prog = getProgressForDrop(drop.id, drop.progress);
            if (!prog) return;

            const required = prog.required_minutes_watched || drop.required_minutes_watched || 0;
            // If claimed, treat it as fully completed for summary purposes
            const current = prog.is_claimed ? required : (prog.current_minutes_watched || 0);

            if (required > 0) {
                totalMinutesRequired += required;
                totalMinutesWatched += Math.min(current, required);
            }

            // Claimable = 100% watched but not claimed
            if (!prog.is_claimed && required > 0 && current >= required) {
                claimableCount++;
            } else if (!prog.is_claimed && current > 0 && required > 0 && current < required) {
                inProgressCount++;
            }
        });
    });

    // Use automation progress if available, otherwise use calculated progress
    const progressPercent = collectProgress
        ? Math.min(100, Math.round((collectProgress.current / collectProgress.required) * 100))
        : totalMinutesRequired > 0
            ? Math.min(100, Math.round((totalMinutesWatched / totalMinutesRequired) * 100))
            : 0;

    // Calculate time remaining for current automation drop
    const timeRemaining = collectProgress
        ? Math.max(0, collectProgress.required - collectProgress.current)
        : 0;

    // Count total active drops and categorize by type
    let timeBasedDropCount = 0;
    let paidDropCount = 0;
    game.active_campaigns.forEach(campaign => {
        campaign.time_based_drops.forEach(drop => {
            const required = drop.required_minutes_watched || 0;
            if (required > 0) {
                timeBasedDropCount++;
            } else {
                // Drops with 0 required minutes are subscription/paid drops
                paidDropCount++;
            }
        });
    });
    const totalActiveDrops = timeBasedDropCount + paidDropCount;

    // Count campaigns
    const campaignCount = game.active_campaigns.length;

    // Dynamic font sizing effect - shrinks font to fit without wrapping.
    // Measures the title's own (flex) box, which already excludes the favorite
    // heart beside it, so the name only shrinks when it genuinely overflows.
    useEffect(() => {
        const titleEl = titleRef.current;
        if (!titleEl) return;

        // Reset to max size to measure
        let currentSize = 14;
        titleEl.style.fontSize = `${currentSize}px`;

        // Shrink font until the text fits its box or we hit the minimum
        const minSize = 9;
        while (titleEl.scrollWidth > titleEl.clientWidth && currentSize > minSize) {
            currentSize -= 0.5;
            titleEl.style.fontSize = `${currentSize}px`;
        }

        setFontSize(currentSize);
    }, [game.name]);

    return (
        <div
            onClick={onClick}
            className={`glass-panel cursor-pointer hover:bg-glass-hover transition-all duration-200 group overflow-hidden relative
                ${isSelected ? 'ring-2 ring-accent' : 'hover:ring-1 hover:ring-accent/40'}
                ${isDropProgressing ? 'ring-2 ring-accent-neon automation-shimmer-overlay' : ''}
            `}
        >
            {/* Image Container */}
            <div className="relative overflow-hidden">
                <img
                    src={boxArtUrl}
                    alt={game.name}
                    className="w-full aspect-[3/4] object-cover group-hover:scale-105 transition-transform duration-200"
                    loading="lazy"
                />

                {/* Top-Left: Status Badges - READY (claimable) or DONE (fully earned) */}
                <div className="absolute top-2 left-2 z-10 flex flex-col gap-1.5">
                    {/* Claimable badge */}
                    {claimableCount > 0 && (
                        <div className="drops-badge-glass-lg !bg-success/50 !border-success/70">
                            <Check size={14} className="text-white" />
                            <span className="text-white">{claimableCount} READY</span>
                        </div>
                    )}
                    {/* Done badge — every drop for this game is already earned. Marks
                        the card so it's not re-opened expecting something to collect.
                        Suppressed while anything is claimable or actively collecting. */}
                    {game.all_drops_claimed && claimableCount === 0 && !isDropProgressing && (
                        <div className="drops-badge-glass-lg !bg-black/45 !border-success/50">
                            <Check size={14} className="text-success" />
                            <span className="text-success">DONE</span>
                        </div>
                    )}
                </div>

                {/* Top-Right: Inventory badge */}
                {game.inventory_items.length > 0 && (
                    <div className="absolute top-2 right-2 z-10 w-7 h-7 rounded-full glass-panel flex items-center justify-center border border-highlight-purple/40 bg-highlight-purple/30">
                        <Package size={14} className="text-white" />
                    </div>
                )}

            </div>

            {/* Chin Section */}
            <div className="p-2" ref={containerRef}>
                {/* Title row: game name (auto-sized to fit) + favorite heart beside it */}
                <div className="flex items-center gap-1.5">
                    <Tooltip content={game.name} delay={400} side="bottom">
                        <h3
                            ref={titleRef}
                            className="flex-1 min-w-0 text-textPrimary font-medium whitespace-nowrap overflow-hidden group-hover:text-accent transition-colors"
                            style={{ fontSize: `${fontSize}px` }}
                        >
                            {game.name}
                        </h3>
                    </Tooltip>

                    {/* Favorite heart - liquid-glass style (matches Home stream cards).
                        Hover-reveal when not favorited, persistent pink once favorited. */}
                    <Tooltip content={isFavorite ? 'Remove from favorites' : 'Add to favorites'} delay={200} side="top">
                        <button
                            onClick={(e) => {
                                e.stopPropagation();
                                if (onToggleFavorite) onToggleFavorite(game.name);
                            }}
                            className="shrink-0 p-0.5 flex items-center justify-center bg-transparent transition-transform duration-300 hover:scale-110 active:scale-95"
                        >
                            <Heart
                                size={16}
                                fill={isFavorite ? 'url(#drops-glass-heart-fill)' : 'none'}
                                stroke={isFavorite ? 'url(#drops-glass-heart-stroke)' : 'currentColor'}
                                strokeWidth={isFavorite ? 1.5 : 2}
                                className={`transition-all duration-300 ${isFavorite ? 'drop-shadow-[0_2px_6px_color-mix(in_srgb,var(--color-highlight-pink)_50%,transparent)]' : 'text-textSecondary opacity-0 group-hover:opacity-100 hover:text-textPrimary'}`}
                            />
                        </button>
                    </Tooltip>
                </div>

                {/* Automation State: Show progress info with drop reward */}
                {isDropProgressing ? (
                    <div className="mt-1.5 space-y-1.5">
                        {/* Drop reward info row */}
                        <div className="flex items-center gap-2">
                            {/* Drop reward image */}
                            {collectDropBenefitImage && (
                                <img
                                    src={collectDropBenefitImage}
                                    alt={collectDropBenefitName}
                                    className="w-8 h-8 rounded object-cover border border-accent-neon/60 shrink-0"
                                />
                            )}
                            <div className="flex-1 min-w-0 space-y-1">
                                {/* Drop name */}
                                <Tooltip content={collectDropBenefitName} delay={300} side="top">
                                    <p className="text-[10px] text-accent-neon truncate w-fit max-w-full">
                                        {collectDropBenefitName}
                                    </p>
                                </Tooltip>
                                {/* Progress bar with stop button */}
                                <div className="flex items-center gap-1.5">
                                    <div className="flex-1 h-1.5 bg-black/30 rounded-full overflow-hidden">
                                        <div
                                            className="h-full automation-progress-bar rounded-full transition-all duration-500"
                                            style={{ width: `${progressPercent}%` }}
                                        />
                                    </div>
                                    {/* Stop button — only when a provider is driving. */}
                                    {externalDropsProvider && (
                                    <Tooltip content="Stop" delay={200} side="top">
                                        <button
                                            onClick={(e) => {
                                                e.stopPropagation();
                                                if (onStopAutomation) onStopAutomation();
                                            }}
                                            className="w-5 h-5 flex items-center justify-center rounded bg-error/20 hover:bg-error/40 border border-transparent shadow-[inset_0_0_0_1px_color-mix(in_srgb,var(--color-error)_40%,transparent)] transition-all shrink-0"
                                        >
                                            <Square size={10} className="text-error" fill="currentColor" />
                                        </button>
                                    </Tooltip>
                                    )}
                                </div>
                            </div>
                        </div>

                        {/* Progress info row */}
                        <div className="flex items-center justify-between text-[10px]">
                            <span className="text-accent-neon font-semibold">
                                {progressPercent}%
                            </span>
                            <span className="flex items-center gap-0.5 text-accent-neon">
                                <Clock size={9} />
                                {formatTimeRemaining(timeRemaining)} left
                            </span>
                        </div>
                    </div>
                ) : (
                    /* Normal State: Show campaign/drop info with Collect All button */
                    <div className="flex items-center justify-between mt-0.5">
                        {game.all_drops_claimed ? (
                            /* Fully earned — show the finished state instead of the
                               (now misleading) live campaign/drop counts. */
                            <span className="flex items-center gap-1 text-success text-xs font-medium min-w-0">
                                <Check size={12} className="shrink-0" />
                                All rewards collected
                            </span>
                        ) : (
                        <div className="flex items-center gap-2 text-textSecondary text-xs min-w-0">
                            {/* Campaign count */}
                            {campaignCount > 0 && (
                                <span className="text-accent">
                                    {campaignCount} campaign{campaignCount !== 1 ? 's' : ''}
                                </span>
                            )}

                            {/* Active drops count */}
                            {totalActiveDrops > 0 && campaignCount > 0 && (
                                <span className="text-textMuted">•</span>
                            )}
                            {totalActiveDrops > 0 && (
                                <span>{totalActiveDrops} drop{totalActiveDrops !== 1 ? 's' : ''}</span>
                            )}

                            {/* In Progress indicator (time-based drops with progress) */}
                            {inProgressCount > 0 && (
                                <Tooltip content="Drops in progress" delay={200} side="bottom">
                                    <span className="flex items-center gap-0.5 text-warning">
                                        <Clock size={10} />
                                        {inProgressCount}
                                    </span>
                                </Tooltip>
                            )}

                            {/* Time-based drops indicator (only show if not showing in-progress) */}
                            {inProgressCount === 0 && timeBasedDropCount > 0 && (
                                <Tooltip content={`${timeBasedDropCount} time-based drop${timeBasedDropCount !== 1 ? 's' : ''}`} delay={200} side="bottom">
                                    <span className="flex items-center gap-0.5 text-info">
                                        <Clock size={10} />
                                        {timeBasedDropCount}
                                    </span>
                                </Tooltip>
                            )}

                            {/* Paid/subscription drops indicator */}
                            {paidDropCount > 0 && (
                                <Tooltip content={`${paidDropCount} subscription/paid drop${paidDropCount !== 1 ? 's' : ''}`} delay={200} side="bottom">
                                    <span className="flex items-center gap-0.5 text-success">
                                        <DollarSign size={10} />
                                        {paidDropCount}
                                    </span>
                                </Tooltip>
                            )}

                            {/* Claimed count (only show if no active drops) */}
                            {game.total_claimed > 0 && !totalActiveDrops && (
                                <span className="text-highlight-purple">
                                    {game.total_claimed} collected
                                </span>
                            )}
                        </div>
                        )}

                        {/* Collect All button removed - users can click into game details to start automation specific campaigns */}
                    </div>
                )}
            </div>
        </div>
    );
}
