// What a badge card shows, for the desktop spotlight card and the phone's badge
// sheet alike: the clicked badge's own facts, filled in by Rust
// `get_badge_detail` (in-memory provider feeds) for third-party badges.
import { useEffect, useState } from 'react';
import { useAppStore } from '../stores/AppStore';
import { getBadgeDetail, type BadgeDetail } from '../services/badgeService';
import {
  badgeProviderLabel,
  isThirdPartyBadgeProvider,
  pickBadgeArt,
  safeBadgeLink,
  type BadgeRef,
} from '../utils/badgeSpotlight';

/** How long a card waits for Rust before showing without its detail, so a
 *  normal (instant) answer lands before the first frame and nothing shifts. */
const DETAIL_WAIT_MS = 250;

/** Rust's detail for a third-party badge (null for the rest, or until it
 *  lands), and whether the card may show yet. */
export function useBadgeDetail(badge: BadgeRef): { detail: BadgeDetail | null; settled: boolean } {
  const thirdParty = isThirdPartyBadgeProvider(badge.provider);
  const [detail, setDetail] = useState<BadgeDetail | null>(null);
  const [settled, setSettled] = useState(!thirdParty);

  useEffect(() => {
    if (!thirdParty) return;
    let live = true;
    const timer = window.setTimeout(() => live && setSettled(true), DETAIL_WAIT_MS);
    const viewer = useAppStore.getState().currentUser?.user_id ?? null;
    getBadgeDetail(badge.provider, badge.id, badge.title, viewer)
      .then((d) => live && setDetail(d))
      .catch(() => {
        // An id Rust refuses still shows the card from the clicked badge.
      })
      .finally(() => live && setSettled(true));
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [thirdParty, badge.provider, badge.id, badge.title]);

  return { detail, settled };
}

export interface BadgeCardFacts {
  title: string;
  providerLabel: string;
  description: string | null;
  art: string;
  website: string | null;
  holders: number | null;
  owned: boolean;
}

export function badgeCardFacts(badge: BadgeRef, detail: BadgeDetail | null, artPx: number): BadgeCardFacts {
  const providerLabel = detail?.provider_label ?? badgeProviderLabel(badge.provider);
  const title = badge.title || detail?.title || providerLabel;
  // Twitch often repeats the title as the description, and some providers send
  // whitespace; neither is worth a line (an empty one still took its margin).
  const rawDescription = (badge.description || detail?.description || '').trim();
  const description = rawDescription && rawDescription.toLowerCase() !== title.trim().toLowerCase() ? rawDescription : null;
  // The art the viewer clicked wins; Rust's fills in when the badge had none.
  const art =
    pickBadgeArt(badge.art, artPx) ||
    pickBadgeArt(
      [detail?.image_1x, detail?.image_2x, detail?.image_4x].filter((u): u is string => !!u).map((url) => ({ url })),
      artPx,
    );
  return {
    title,
    providerLabel,
    description,
    art,
    website: safeBadgeLink(detail?.click_url ?? badge.clickUrl ?? detail?.homepage),
    holders: detail?.holder_count ?? null,
    owned: detail?.owned === true,
  };
}
