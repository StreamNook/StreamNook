import { lazy, Suspense } from 'react';
import { AnimatePresence } from 'framer-motion';
import { useBadgeSpotlightStore } from '../stores/badgeSpotlightStore';
import type { BadgeVersion } from '../utils/badgeLinkOpen';

const BadgesOverlay = lazy(() => import('./BadgesOverlay'));
const BadgeDetailOverlay = lazy(() => import('./BadgeDetailOverlay'));

/**
 * What App's badge slot shows: a badge detail, or the placeholder a badge link opened at once and
 * the lookup will fill (`read` numbers the lookup, so only its own answer applies).
 */
export type SelectedBadge =
  | { kind: 'badge'; badge: BadgeVersion; setId: string }
  | { kind: 'link'; key: string; state: 'loading' | 'missing' | 'failed'; read: number }
  | null;

/**
 * What a link that opens a different screen runs first: the badge detail or placeholder, the Badges
 * list behind it (opening a badge link also opens Badges), and a badge spotlight card, so none of
 * them stays over the new screen.
 */
// eslint-disable-next-line react-refresh/only-export-components
export function makeCloseLinkSheet(deps: {
  setSelectedBadge: (next: null) => void;
  setShowBadgesOverlay: (open: boolean) => void;
}): () => void {
  return () => {
    deps.setSelectedBadge(null);
    deps.setShowBadgesOverlay(false);
    useBadgeSpotlightStore.getState().close();
  };
}

export interface BadgeOverlayInitial {
  paintId: string | null;
  badgeId: string | null;
  streamNook: boolean | string;
  target: { tab: string; query?: string } | null;
}

interface BadgeDetailSlotProps {
  selectedBadge: SelectedBadge;
  setSelectedBadge: (next: SelectedBadge) => void;
  showBadgesOverlay: boolean;
  setShowBadgesOverlay: (open: boolean) => void;
  overlayInitial: BadgeOverlayInitial;
  /** Try again on a failed link placeholder. */
  onRetryBadgeLink: (key: string) => void;
}

/**
 * Both badge mounts of App: the Badges list while nothing is selected, and the badge detail (or its
 * link placeholder) once something is. Showing a detail unmounts the list, which is why the
 * placeholder lives here and not inside the list.
 */
export default function BadgeDetailSlot({
  selectedBadge,
  setSelectedBadge,
  showBadgesOverlay,
  setShowBadgesOverlay,
  overlayInitial,
  onRetryBadgeLink,
}: BadgeDetailSlotProps) {
  return (
    <>
      <AnimatePresence>
        {showBadgesOverlay && !selectedBadge && (
          <Suspense fallback={null}>
            <BadgesOverlay
              onClose={() => setShowBadgesOverlay(false)}
              onBadgeClick={(badge, setId) => setSelectedBadge({ kind: 'badge', badge, setId })}
              initialPaintId={overlayInitial.paintId}
              initialBadgeId={overlayInitial.badgeId}
              initialStreamNook={overlayInitial.streamNook}
              initialTarget={overlayInitial.target}
            />
          </Suspense>
        )}
      </AnimatePresence>
      <AnimatePresence>
        {selectedBadge && (
          <Suspense fallback={null}>
            <BadgeDetailOverlay
              selected={selectedBadge}
              onClose={() => {
                setSelectedBadge(null);
                setShowBadgesOverlay(false);
              }}
              onBack={() => setSelectedBadge(null)}
              onRetry={onRetryBadgeLink}
            />
          </Suspense>
        )}
      </AnimatePresence>
    </>
  );
}
