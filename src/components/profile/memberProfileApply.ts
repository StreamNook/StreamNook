// What the member card's loader writes into its state, decided as plain data so
// it can be tested without React. `useMemberProfile` (memberProfile.ts) calls
// these and applies the answer with its setters.
//
// Two rules live here. A late first-paint card (Rust's `get_profile_snapshot`)
// fills only the fields no live source has set yet. And a live read that
// FAILED never counts as live: only an answered read may block the card or
// replace what it drew.

import type { CSSProperties } from 'react';

// The answer of the Rust command `get_profile_snapshot`, as it serializes.
export interface FirstPaintIdentity {
  login: string;
  displayName: string;
  /** A Twitch CDN picture, else empty. */
  avatar: string;
}

export interface FirstPaintBadge {
  src: string;
  title: string;
}

export interface FirstPaintThirdPartyBadge extends FirstPaintBadge {
  provider: string;
  key?: string;
}

export interface FirstPaintCard {
  profileTheme: string; // 'tier' | 'paint' | <atmosphere id>
  hiddenSections: string[];
  memberNumber: number | null;
  cosmeticSlug: string | null;
  /** The name's paint as CSS properties; images only from 7TV's CDN. */
  namePaint: Record<string, string | number> | null;
  /** The worn 7TV badge, in the shape the badge row reads. */
  seventvBadge: { id: string; name: string; tooltip: string } | null;
  wornBadges: {
    twitch: FirstPaintBadge | null;
    thirdParty: FirstPaintThirdPartyBadge[];
    bttvPro: FirstPaintBadge | null;
  };
  counts: { paints: number; badges: number; sn: number };
}

export interface FirstPaint {
  identity: FirstPaintIdentity;
  /** Null when only the identity is known (no row the server wrote). */
  card: FirstPaintCard | null;
}

/** Which fields a live source has already set; a first paint never overwrites them. */
export interface LiveFlags {
  info: boolean;
  counts: boolean;
  paint: boolean;
  hidden: boolean;
  badges: boolean;
  theme: boolean;
}

export const noLive = (): LiveFlags => ({
  info: false,
  counts: false,
  paint: false,
  hidden: false,
  badges: false,
  theme: false,
});

export interface FirstPaintPlan {
  /** Each field is set only when present (undefined leaves the state alone). */
  info?: FirstPaintIdentity;
  counts?: FirstPaintCard['counts'];
  namePaint?: CSSProperties | null;
  hiddenSections?: string[];
  wornBadges?: {
    seventv: FirstPaintCard['seventvBadge'];
    twitch: FirstPaintBadge | null;
    thirdParty: FirstPaintThirdPartyBadge[];
    bttvPro: FirstPaintBadge | null;
  };
  /** Resolved once the atmosphere catalog is ready, if still no live theme. */
  theme?: { profileTheme: string; namePaint: CSSProperties | null };
  /** The identity is on screen: the loading state can end. */
  endLoading: boolean;
}

/**
 * What a first paint may write. `previewOwnsBadges`: the Settings live preview
 * re-resolves the badge row on each loadout edit, so a late card must not
 * clobber it.
 */
export function planFirstPaint(snap: FirstPaint | null, live: LiveFlags, previewOwnsBadges: boolean): FirstPaintPlan {
  const plan: FirstPaintPlan = { endLoading: false };
  if (!snap) return plan;
  if (!live.info) {
    plan.info = snap.identity;
    plan.endLoading = true;
  }
  const card = snap.card;
  if (!card) return plan;
  const paint = card.namePaint as CSSProperties | null;
  if (!live.counts) plan.counts = card.counts;
  if (!live.paint) plan.namePaint = paint;
  if (!live.hidden) plan.hiddenSections = card.hiddenSections;
  if (!live.badges && !previewOwnsBadges) {
    plan.wornBadges = {
      seventv: card.seventvBadge,
      twitch: card.wornBadges.twitch,
      thirdParty: card.wornBadges.thirdParty,
      bttvPro: card.wornBadges.bttvPro,
    };
  }
  if (!live.theme) plan.theme = { profileTheme: card.profileTheme, namePaint: paint };
  return plan;
}

/** What `getProfilePrefs` answers. `ok` is false when the read failed. */
export interface PrefsRead {
  ok: boolean;
  profileTheme: string;
  hiddenSections?: string[];
}

export interface PrefsPlan {
  /** Flags to set on the live record. */
  live: Partial<LiveFlags>;
  hiddenSections?: string[];
  /** A theme the prefs settle without the paint (an atmosphere or the tier). */
  theme?: { profileTheme: string };
}

/**
 * The prefs read, applied. A failed read (null, or `ok` not true) sets nothing
 * and marks nothing live, so the first-paint card still shows what the member
 * hides and wears.
 */
export function planPrefs(prefs: PrefsRead | null): PrefsPlan {
  if (!prefs || prefs.ok !== true) return { live: {} };
  const plan: PrefsPlan = { live: { hidden: true }, hiddenSections: prefs.hiddenSections ?? [] };
  if (prefs.profileTheme !== 'paint') {
    plan.live.theme = true;
    plan.theme = { profileTheme: prefs.profileTheme };
  }
  return plan;
}

export interface PaintThemePlan {
  live: Partial<LiveFlags>;
  /** 'paint' = draw the paint theme; null = no theme (the member no longer wears a paint). */
  theme: 'paint' | null;
}

/** What the live 7TV cosmetics read told us about the member's paint. */
export interface LivePaint {
  /** The read genuinely answered (a real answer, including "wears no paint"), not a failed fetch. */
  answered: boolean;
  /** The member wears a paint. */
  hasPaint: boolean;
}

/**
 * The one theme that waits for the live 7TV paint: the member chose their
 * paint as the theme. With a paint it draws. Without one it is cleared (and
 * marked live), so a stale card cannot keep a paint the member took off, but
 * only when 7TV answered: after a failed read it does nothing and leaves the
 * theme unset, so the first-paint card's paint aura stays. Null when the prefs
 * failed or name another theme.
 */
export function planPaintTheme(prefs: PrefsRead | null, paint: LivePaint): PaintThemePlan | null {
  if (!prefs || prefs.ok !== true || prefs.profileTheme !== 'paint') return null;
  if (paint.hasPaint) return { live: { theme: true }, theme: 'paint' };
  if (!paint.answered) return null;
  return { live: { theme: true }, theme: null };
}

/** What the paint read decides for the name's paint. */
export type NamePaintPlan = { set: false } | { set: true; value: CSSProperties | null };

/**
 * The name's paint from the live 7TV read. Only an answered read sets it, with
 * the paint or with null for a member who wears none. After a failed read it
 * does nothing, so the first-paint card's name paint stays and `live.paint`
 * stays unset.
 */
export function planNamePaint(read: { answered: boolean; paintStyle: CSSProperties | null }): NamePaintPlan {
  return read.answered ? { set: true, value: read.paintStyle } : { set: false };
}
