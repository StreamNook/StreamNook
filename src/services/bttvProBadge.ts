// BetterTTV Pro badge — the opt-in identity piece that can't ride the normal
// loadout resolver.
//
// Every other third-party badge (FFZ / Chatterino / Homies / Chatsen / Chatty /
// DankChat + BTTV contributor) is resolved SERVER-SIDE by the Identity API from a
// public per-provider list. BTTV Pro is different: it is delivered only over
// BetterTTV's WebSocket and has no public REST list, so the server physically
// can't resolve it. We therefore resolve it CLIENT-SIDE, per user, and store only
// a sentinel key in the loadout.
//
// The sentinel key uses the UPPERCASE `BTTV` provider on purpose: the server's
// resolver keys are lowercase, so it harmlessly ignores `BTTV:bttv-pro` (it never
// tries to resolve it), AND it matches the profile card's existing Pro badge
// object key (`${provider}:${id}`), so the same key works in the editor, the
// profile card, and chat with no per-surface special-casing.

import { invoke } from '@tauri-apps/api/core';
import type { ThirdPartyBadge } from './badgeService';

/** The loadout key that means "show my BetterTTV Pro badge." */
export const BTTV_PRO_LOADOUT_KEY = 'BTTV:bttv-pro';

/** Stable id of the Pro badge within a rendered badge list (for keys + dedupe). */
export const BTTV_PRO_BADGE_ID = 'bttv-pro';

/** Shape a resolved Pro badge URL into the renderer's ThirdPartyBadge. Provider
 *  + id are chosen so `${provider}:${id}` === BTTV_PRO_LOADOUT_KEY. */
export function buildBttvProBadge(url: string): ThirdPartyBadge {
  return {
    id: BTTV_PRO_BADGE_ID,
    title: 'BTTV Pro',
    imageUrl: url,
    image1x: url,
    image2x: url,
    image4x: url,
    provider: 'BTTV',
  };
}

/** Adds the Pro badge to a third-party list that is already in the fixed
 *  provider order (Rust's THIRD_PARTY_ORDER: FFZ, then BetterTTV, then the
 *  rest). Pro is the one badge placed here rather than in Rust, because it
 *  arrives after the list does; it goes after the FFZ and BetterTTV badges. */
export function withBttvPro<T extends { provider?: string }>(list: T[], pro: T): T[] {
  let at = 0;
  list.forEach((b, i) => {
    const p = (b.provider ?? '').toLowerCase();
    if (p === 'ffz' || p === 'bttv') at = i + 1;
  });
  return [...list.slice(0, at), pro, ...list.slice(at)];
}

/** Resolve a user's CURRENT BTTV Pro badge URL (it progresses with tenure), or
 *  null if they don't have Pro. Backed by the BTTV socket the app already keeps
 *  open; failures resolve to null so a hiccup never breaks badge rendering. */
export async function resolveBttvProUrl(userId: string): Promise<string | null> {
  try {
    const badge = await invoke<{ url: string; started_at: string | null; glow: boolean } | null>(
      'get_bttv_pro_badge',
      { userId },
    );
    return badge?.url ?? null;
  } catch {
    return null;
  }
}
