// Ordering for the Rewards badge gallery.
//
// Everything derives from ONE newest-first list built with ONE comparator. The
// comparator this replaced switched rules per pair (rank when both badges had
// one, date otherwise), which is not a consistent order, and a sort handed an
// inconsistent comparator scrambles; every sort used it as the tiebreak, so all
// of them scrambled.
import type { GlobalBadge } from './badgeGalleryStore';

export type BadgeSort = 'newest' | 'oldest' | 'available' | 'soon' | 'usage';

/**
 * Newest first by date added. The precomputed rank is derived from the same
 * date, so it only breaks ties within a day (it orders those by usage). A badge
 * with no date yet sorts last, and the key keeps the order stable.
 */
const newestCmp = (a: GlobalBadge, b: GlobalBadge) =>
  b.addedMs - a.addedMs || a.position - b.position || a.key.localeCompare(b.key);

export function orderBadges(badges: GlobalBadge[], sort: BadgeSort): GlobalBadge[] {
  const newest = [...badges].sort(newestCmp);
  const lead = (match: (b: GlobalBadge) => boolean) => [
    ...newest.filter(match),
    ...newest.filter((b) => !match(b)),
  ];
  switch (sort) {
    case 'oldest':
      return newest.reverse();
    case 'available':
      return lead((b) => b.status === 'available');
    case 'soon':
      return lead((b) => b.status === 'coming-soon');
    case 'usage': {
      const at = new Map(newest.map((b, i) => [b.key, i]));
      return [...newest].sort((a, b) => b.usage - a.usage || at.get(a.key)! - at.get(b.key)!);
    }
    default:
      return newest;
  }
}
