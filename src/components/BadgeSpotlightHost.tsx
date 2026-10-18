// Mount point for the badge spotlight card and the emote card (an emote
// clicked in chat). Every window that shows badges
// (main, the MultiChat popout, the chat overlay, profile-card windows) renders
// one, because each window has its own store: a host missing from a window
// makes every badge click there do nothing. The card's code loads on the first
// click, not at window start.
import { lazy, Suspense } from 'react';
import { useBadgeSpotlightStore } from '../stores/badgeSpotlightStore';
import { useEmoteCardStore } from '../stores/emoteCardStore';

const BadgeSpotlight = lazy(() => import('./BadgeSpotlight'));
const EmoteCard = lazy(() => import('./EmoteCard'));

export default function BadgeSpotlightHost() {
  const badgeOpen = useBadgeSpotlightStore((s) => s.badge !== null);
  const emoteOpen = useEmoteCardStore((s) => s.emote !== null);
  if (!badgeOpen && !emoteOpen) return null;
  return (
    <Suspense fallback={null}>
      {badgeOpen && <BadgeSpotlight />}
      {emoteOpen && <EmoteCard />}
    </Suspense>
  );
}
