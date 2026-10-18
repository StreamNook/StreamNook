// The badge the spotlight card is showing, per window. Every Tauri window has
// its own JS context, so a click in a popout opens the card in that popout.
// Twitch, 7TV and StreamNook badges skip the card: they have a full page in the
// Badges overlay, so a click goes straight there (in main; a popout raises it).
// Third-party and Kick / YouTube / TikTok badges have no such page, so the card
// is where their details live.
import { create } from 'zustand';
import { useTooltipStore } from './TooltipStore';
import { directBadgeRoute, openBadgeDetailsRoute, type BadgeRef } from '../utils/badgeSpotlight';

interface BadgeSpotlightState {
  badge: BadgeRef | null;
  /** Opened from inside the Badges overlay, so "Open in Badges" would only
   *  lead back to where the viewer already is. */
  fromBadges: boolean;
  open: (badge: BadgeRef, opts?: { fromBadges?: boolean }) => void;
  close: () => void;
}

export const useBadgeSpotlightStore = create<BadgeSpotlightState>((set) => ({
  badge: null,
  fromBadges: false,
  open: (badge, opts) => set({ badge, fromBadges: !!opts?.fromBadges }),
  close: () => set({ badge: null, fromBadges: false }),
}));

/** Show `badge` in this window's spotlight card. The hover card under the
 *  pointer goes first: the modal covers its trigger, so no mouseleave would
 *  ever close it. */
export function openBadgeSpotlight(badge: BadgeRef, opts?: { fromBadges?: boolean }): void {
  useTooltipStore.getState().hideTooltip();
  useBadgeSpotlightStore.getState().open(badge, opts);
}

/** What a badge click does: its page in the Badges overlay where it has one
 *  (Twitch, 7TV, StreamNook), else the spotlight card in this window. */
export function openBadgeInfo(badge: BadgeRef): void {
  const route = directBadgeRoute(badge);
  if (!route) {
    openBadgeSpotlight(badge);
    return;
  }
  useTooltipStore.getState().hideTooltip();
  void openBadgeDetailsRoute(route);
}
