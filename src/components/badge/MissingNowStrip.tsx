import { useEffect, useRef, useState } from 'react';
import { ArrowUpRight, ChevronDown } from 'lucide-react';
import { CardChip } from '../ui/CardChip';
import { useAppStore } from '../../stores/AppStore';
import {
  closesInLabel,
  timeLeftLabel,
  type BadgeStanding,
  type MissingBadge,
} from '../../services/badgeStanding';
import { CatchUpPanel } from './CatchUpPanel';
import { EarnChips, RandomDrawNote } from './EarnChips';

// Every card is the same height, so a collapsed list always ends on a row
// boundary instead of slicing a card in half.
const CARD_HEIGHT_PX = 64;
const GAP_PX = 12;
const COLLAPSED_ROWS = 2;
const COLLAPSED_MAX_PX = COLLAPSED_ROWS * CARD_HEIGHT_PX + (COLLAPSED_ROWS - 1) * GAP_PX;

/** The badges the signed-in account does not own that can be earned right now,
 *  pinned above the Twitch badge grid. Everything here comes ready from Rust
 *  (`get_badge_standing`); this only lays it out. */
export const MissingNowStrip = ({
  standing,
  onOpenBadge,
}: {
  standing: BadgeStanding | null;
  onOpenBadge: (key: string) => void;
}) => {
  const [expanded, setExpanded] = useState(false);
  // Whether the cards need more than the collapsed rows at the panel's current
  // width. Measured, because how many cards fit on a row depends on the width.
  const [overflows, setOverflows] = useState(false);
  const gridRef = useRef<HTMLDivElement>(null);

  const missing = standing?.missing_now ?? [];
  const loading =
    !standing || !standing.catalogue_ready || (standing.collection === 'partial' && standing.collection_reason !== 'fetch_failed');
  const failed = standing?.collection === 'partial' && standing.collection_reason === 'fetch_failed';
  const showGrid = !loading && !failed && missing.length > 0;
  // Rust sorts the list soonest-closing first.
  const soonest = showGrid ? closesInLabel(missing[0].ends_ms) : '';

  useEffect(() => {
    const el = gridRef.current;
    if (!el) return;
    // Fires once on observe, then on every resize of the grid.
    const observer = new ResizeObserver(() => setOverflows(el.scrollHeight > COLLAPSED_MAX_PX + 1));
    observer.observe(el);
    return () => observer.disconnect();
  }, [showGrid, missing.length]);

  if (standing?.collection === 'unavailable') return null;

  return (
    <section className="mb-6">
      {/* The totals sit to the right and drop under the title when the panel
          is too narrow for both. */}
      <header className="flex flex-wrap items-center gap-x-4 gap-y-3">
        <div className="min-w-[14rem] flex-1">
          <div className="flex items-center gap-2">
            <h3 className="missing-now-title">Missing, earnable now</h3>
            {showGrid && <CardChip kind="ready">{missing.length}</CardChip>}
          </div>
          <p className="mt-0.5 flex items-center gap-1.5 text-[11.5px] text-textMuted truncate">
            {standing?.refreshing ? (
              <>
                <span aria-hidden className="missing-now-pulse" />
                <span>Checking Twitch</span>
              </>
            ) : (
              <>
                {soonest && (
                  <span>
                    The next one closes <span className="font-medium text-textSecondary">{soonest}</span>
                  </span>
                )}
                {standing?.stale && standing.collection === 'complete' && (
                  <span>{soonest ? '· ' : ''}Last checked earlier</span>
                )}
              </>
            )}
          </p>
        </div>
        {showGrid && standing?.catch_up && <CatchUpPanel totals={standing.catch_up} />}
      </header>
      <div aria-hidden className="missing-now-rule" />

      {loading ? (
        <div
          className="rounded-xl bg-white/[0.03] border border-white/[0.05] animate-pulse"
          style={{ height: CARD_HEIGHT_PX }}
        />
      ) : failed ? (
        <p className="text-sm text-textSecondary">Couldn't read your badges from Twitch. Refresh to try again.</p>
      ) : missing.length === 0 ? (
        <p className="text-sm text-textSecondary">You have every badge that's earnable right now.</p>
      ) : (
        <>
          {/* Wraps onto new rows rather than scrolling sideways, with columns
              stretched so each row fills the panel evenly. A long list shows
              its first rows until expanded. */}
          <div
            ref={gridRef}
            className="grid grid-cols-[repeat(auto-fill,minmax(280px,1fr))] gap-3 overflow-hidden"
            style={{ gridAutoRows: CARD_HEIGHT_PX, maxHeight: expanded ? undefined : COLLAPSED_MAX_PX }}
          >
            {missing.map((badge) => (
              <MissingCard
                key={badge.key}
                badge={badge}
                icons={standing?.earn_icons}
                onOpen={() => onOpenBadge(badge.key)}
              />
            ))}
          </div>
          {(overflows || expanded) && (
            <button
              onClick={() => setExpanded((v) => !v)}
              className="mt-2.5 flex items-center gap-1 text-xs font-medium text-textSecondary hover:text-accent transition-colors"
            >
              {expanded ? 'Show fewer' : `Show all ${missing.length}`}
              <ChevronDown size={13} className={`transition-transform ${expanded ? 'rotate-180' : ''}`} />
            </button>
          )}
        </>
      )}
    </section>
  );
};

const MissingCard = ({
  badge,
  icons,
  onOpen,
}: {
  badge: MissingBadge;
  icons?: Record<string, string>;
  onOpen: () => void;
}) => {
  const category = badge.category;
  const left = timeLeftLabel(badge.ends_ms);

  const openCategory = () => {
    if (!category) return;
    const { setShowBadgesOverlay, navigateToCategoryByName } = useAppStore.getState();
    setShowBadgesOverlay(false);
    void navigateToCategoryByName(category);
  };

  return (
    <div
      className="group min-w-0 h-full flex items-center gap-3 rounded-xl bg-white/[0.04] hover:bg-white/[0.07] border border-white/[0.06] px-3 transition-colors"
      title={badge.earn.detail ?? undefined}
    >
      <button onClick={onOpen} className="shrink-0" aria-label={`${badge.title} details`}>
        <img
          src={badge.image_url}
          alt=""
          className="w-10 h-10 object-contain drop-shadow-[0_0_10px_color-mix(in_srgb,var(--color-success)_35%,transparent)]"
          loading="lazy"
          draggable={false}
        />
      </button>
      <div className="flex-1 min-w-0">
        {/* The title and how it is earned share the top line; the chips keep
            their width and the title gives way, truncating with a tooltip. */}
        <div className="flex items-center gap-2 min-w-0">
          <button
            onClick={onOpen}
            title={badge.title}
            className="min-w-[3.5rem] flex-1 text-left text-[13px] font-medium text-textPrimary truncate group-hover:text-accent transition-colors"
          >
            {badge.title}
          </button>
          <EarnChips earn={badge.earn} icons={icons} />
        </div>
        <div className="flex items-center gap-2 mt-1">
          {left && <span className="shrink-0 whitespace-nowrap text-[11px] font-medium text-success">{left}</span>}
          {badge.earn.random_of && <RandomDrawNote of={badge.earn.random_of} />}
          {category && (
            <button
              onClick={openCategory}
              className="flex items-center gap-0.5 min-w-0 text-[11px] text-textMuted hover:text-accent transition-colors"
            >
              <span className="truncate">{category}</span>
              <ArrowUpRight size={11} className="shrink-0" />
            </button>
          )}
        </div>
      </div>
    </div>
  );
};
