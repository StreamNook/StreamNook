// A StreamNook member's public profile: everything their member card shows
// (the chat user card for a member, and the same card as your own live preview
// in Settings), loaded one way.
//
// Loads stale-while-revalidate: the card streamnook.app wrote for the member
// (read once in Rust, or just their identity when there is none) paints first,
// then the live sources (Helix identity, cosmetics, worn badges, the profile
// theme) replace it. A live value always wins over the first paint, however late
// it lands.

import { useEffect, useState, useSyncExternalStore, type CSSProperties } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { helixGet } from '../../services/helix';
import { useAppStore } from '../../stores/AppStore';
import { Logger } from '../../utils/logger';
import { getTierAccent, getWornBadgeCaption } from '../StreamNookBadge';
import { getFullProfileWithFallback } from '../../services/cosmeticsCache';
import { computePaintStyle } from '../../services/seventvService';
import { getAtmosphere, type Atmosphere } from '../../services/atmospheres';
import { getResolvedIdentity, getIdentityWithCache, isIdentityFallback } from '../../services/identityService';
import { resolveBttvProUrl, BTTV_PRO_LOADOUT_KEY } from '../../services/bttvProBadge';
import {
  getStreamNookUserNumber,
  getOwnedCosmeticSlugs,
  isCosmeticsRegistryLoaded,
  getProfilePrefs,
  subscribeStreamNookRegistryVersion,
  getStreamNookRegistryVersion,
  subscribeCosmeticsVersion,
  getCosmeticsVersion,
  whenAtmospheresReady,
  incrementProfileView,
  getProfileViews,
} from '../../services/supabaseService';
import {
  noLive,
  planFirstPaint,
  planNamePaint,
  planPaintTheme,
  planPrefs,
  PREVIEW_OTHERS,
  sevenTvBadgesStep,
  sevenTvCountsStep,
  type FirstPaint,
  type WornBadgeAnswers,
} from './memberProfileApply';

// "#rrggbb" -> "r, g, b" for use inside rgba(...).
export const hexToRgb = (hex: string): string | null => {
  const m = /^#?([0-9a-fA-F]{6})$/.exec(hex.trim());
  if (!m) return null;
  const n = parseInt(m[1], 16);
  return `${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}`;
};

// Resolve a global Twitch badge "set/version" key to an image. Global badges are
// universal, so this resolves for any member regardless of the viewer. Warms the
// Rust global-badge cache if it's cold.
export const resolveGlobalTwitchBadge = async (
  setVer: string,
): Promise<{ src: string; title: string } | null> => {
  const slash = setVer.indexOf('/');
  if (slash < 0) return null;
  const setId = setVer.slice(0, slash);
  const version = setVer.slice(slash + 1);
  try {
    let global = await invoke<any>('get_cached_global_badges');
    if (!global?.data) {
      await invoke('prefetch_global_badges').catch(() => {});
      global = await invoke<any>('get_cached_global_badges');
    }
    const set = global?.data?.find((s: any) => s.set_id === setId);
    const v = set?.versions?.find((ver: any) => ver.id === version);
    if (v) {
      return { src: v.image_url_4x || v.image_url_2x || v.image_url_1x, title: v.title || 'Twitch' };
    }
  } catch { /* cache cold / offline — badge just won't resolve */ }
  return null;
};

export interface TargetInfo {
  login: string;
  displayName: string;
  avatar: string;
}

export type ResolvedTheme = { accentRgb: string | null; paintAura?: CSSProperties; atmosphere?: Atmosphere };

// Build the resolved background theme from a member's profile_theme id + their
// resolved 7TV paint style (ps). Shared by the live resolve AND the snapshot
// hydrate so both paint identically (no flicker when the revalidate lands).
// Assumes the atmosphere catalog is loaded (getAtmosphere is a sync registry read).
export const resolveProfileTheme = (
  profileTheme: string,
  ps: CSSProperties | null,
): ResolvedTheme | null => {
  if (profileTheme === 'paint' && ps) {
    // Blur the paint into a soft colored ambiance (not a pixelated stretch);
    // animated paints still shimmer through.
    const hexes =
      typeof ps.backgroundImage === 'string' ? ps.backgroundImage.match(/#[0-9a-fA-F]{6}/g) : null;
    const repHex = hexes && hexes.length ? hexes[Math.floor(hexes.length / 2)] : null;
    return {
      accentRgb: repHex ? hexToRgb(repHex) : null,
      paintAura: {
        backgroundImage: ps.backgroundImage,
        backgroundColor:
          typeof ps.backgroundColor === 'string' && !ps.backgroundColor.startsWith('var')
            ? ps.backgroundColor
            : undefined,
        backgroundSize: 'cover',
        backgroundPosition: 'center',
        backgroundRepeat: 'no-repeat',
        filter: 'blur(40px) saturate(1.3)',
      },
    };
  }
  const atm = getAtmosphere(profileTheme);
  if (atm) return { accentRgb: atm.accent, atmosphere: atm };
  return null;
};

export type WornBadges = {
  seventv: any | null;
  twitch: { src: string; title: string } | null;
  thirdParty: any[];
  bttvPro: { src: string; title: string } | null;
};

// Resolve the member's worn identity badge row the SAME way chat does, so the
// overlay shows their full cross-client combination: third-party badges from the
// ownership-checked server bundle, BTTV Pro from the socket (the server drops
// that key), the Twitch global badge from the loadout's `twitch:<...>` key, and
// the active 7TV badge. Shared by the initial load AND the live-preview re-resolve
// (a loadout edit), so both paths produce identical results. `profile` is the
// result of getFullProfileWithFallback (carries the 7TV cosmetics).
export const resolveWornBadges = async (
  userId: string,
  profile: any,
): Promise<WornBadges & { answers: WornBadgeAnswers }> => {
  const seventv =
    (profile?.seventvCosmetics?.badges as any[] | undefined)?.find((b) => b?.selected) ?? null;
  let twitch: { src: string; title: string } | null = null;
  let thirdParty: any[] = [];
  let bttvPro: { src: string; title: string } | null = null;
  // Whether each part is a real answer, each from its own read. The reads below
  // stand in with empty defaults when they fail, which look like a member
  // wearing nothing, so a failure they show is tracked here and never marks
  // that part live.
  let answers: WornBadgeAnswers = { twitch: false, thirdParty: false, bttv: false };
  try {
    const [resolved, rawLoadout, liveTwitch] = await Promise.all([
      getResolvedIdentity(userId).catch(() => null),
      getIdentityWithCache(userId).catch(() => null),
      // The badge they wear on Twitch now, as chat shows it (Rust asks
      // Twitch). Undefined when Twitch did not answer.
      invoke<{ src: string; title: string } | null>('get_selected_twitch_badge', { userId }).catch(
        () => undefined,
      ),
    ]);
    thirdParty = (resolved?.badges ?? [])
      .filter((b: any) => b.provider !== 'twitch')
      .map((b: any) => ({ key: b.key, provider: b.provider, title: b.title, src: b.image_url }));
    const keys: string[] = rawLoadout?.badges ?? [];
    // The loadout's `twitch:` key is only the copy the app took when that
    // member last opened Profile settings, so it stands in only when Twitch
    // did not answer.
    const twitchKey = keys.find((k) => k.startsWith('twitch:'));
    if (liveTwitch !== undefined) {
      twitch = liveTwitch;
    } else if (twitchKey) {
      const v = twitchKey.slice('twitch:'.length);
      if (v.includes('/')) {
        // Legacy set/version key — resolve via the global-badge cache.
        twitch = await resolveGlobalTwitchBadge(v);
      } else if (v) {
        // Image-id key — the universal Twitch badge CDN URL renders the same
        // badge for any viewer (global or channel-scoped), no cache.
        twitch = { src: `https://static-cdn.jtvnw.net/badges/v1/${v}/3`, title: 'Twitch' };
      }
    }
    if (keys.includes(BTTV_PRO_LOADOUT_KEY)) {
      const url = await resolveBttvProUrl(userId).catch(() => null);
      if (url) bttvPro = { src: url, title: 'BTTV Pro' };
    }
    answers = {
      // Only Twitch itself answers; the loadout's copy may fill the badge in
      // but is not an answer, so a card arriving later still wins over it.
      twitch: liveTwitch !== undefined,
      thirdParty: resolved !== null && !isIdentityFallback(resolved),
      // Whether the member wears BTTV Pro is read from the loadout.
      bttv: rawLoadout !== null && !isIdentityFallback(rawLoadout),
    };
  } catch { /* badges optional; nothing counts as answered */ }
  return { seventv, twitch, thirdParty, bttvPro, answers };
};

/** A live preview of the viewer's own profile (Settings edits): overrides the
 *  fetched hidden sections and theme so edits reflect instantly. */
export interface MemberProfilePreview {
  hiddenSections: string[];
  profileTheme: string;
  badgeRevision: number;
}

/**
 * Load a member's public profile and derive how it is themed.
 *
 * `followsLivePreview`: true only for your own card as the Settings live
 * preview; every other card passes false and a null `preview`.
 */
export function useMemberProfile(
  userId: string | null | undefined,
  preview: MemberProfilePreview | null,
  followsLivePreview: boolean,
) {
  // The VIEWER's own id, so we don't count self-views (or the live preview).
  const currentUserId = useAppStore((s) => s.currentUser?.user_id);

  const [info, setInfo] = useState<TargetInfo | null>(null);
  const [counts, setCounts] = useState({ paints: 0, badges: 0, sn: 0 });
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(false);
  // The viewed member's active 7TV paint paints their NAME (free, like chat).
  const [namePaint, setNamePaint] = useState<CSSProperties | null>(null);
  // The worn 7TV paint itself (for the paint chip) and the member's 7TV
  // account (for the 7TV profile button). Filled by the live resolve.
  const [activePaint, setActivePaint] = useState<({ id: string; name: string } & Record<string, unknown>) | null>(null);
  const [seventvUserId, setSeventvUserId] = useState<string | null>(null);
  // The resolved premium profile background theme (7TV paint or a StreamNook
  // Atmosphere). null = the free tier aura.
  const [theme, setTheme] = useState<{
    accentRgb: string | null;
    paintAura?: CSSProperties; // when the source is the 7TV paint
    atmosphere?: Atmosphere; // when the source is a StreamNook Atmosphere
  } | null>(null);
  // Sections the viewed member hid from their public profile.
  const [hiddenSections, setHiddenSections] = useState<string[]>([]);
  // The identity badges the member is wearing (active 7TV badge + chosen
  // third-party loadout). The StreamNook badge is rendered from the registry.
  const [wornBadges, setWornBadges] = useState<{
    seventv: any | null;
    twitch: { src: string; title: string } | null;
    thirdParty: any[];
    bttvPro: { src: string; title: string } | null;
  }>({ seventv: null, twitch: null, thirdParty: [], bttvPro: null });
  // The viewed member's public profile-view count. null = unknown / unavailable.
  const [profileViews, setProfileViews] = useState<number | null>(null);

  // Record a profile view (and read the current count for display). Counting is
  // gated: never your own profile, never the live preview, and deduped per
  // session inside incrementProfileView. The hide toggle only affects DISPLAY
  // (below), so a hidden count still accumulates.
  useEffect(() => {
    let alive = true;
    (async () => {
      if (!userId) {
        if (alive) setProfileViews(null);
        return;
      }
      const shouldCount = !!currentUserId && userId !== currentUserId && !preview;
      const counted = shouldCount ? await incrementProfileView(userId) : null;
      const n = counted ?? (await getProfileViews(userId));
      if (alive) setProfileViews(n);
    })();
    return () => { alive = false; };
    // currentUserId is stable per session; only re-run on the viewed profile.
  }, [userId]);

  // Re-render when the StreamNook member / cosmetics registries update so the
  // tier card and counts resolve for the viewed user once loaded.
  useSyncExternalStore(subscribeStreamNookRegistryVersion, getStreamNookRegistryVersion, getStreamNookRegistryVersion);
  useSyncExternalStore(subscribeCosmeticsVersion, getCosmeticsVersion, getCosmeticsVersion);

  // Reset the viewed-profile state the moment the target changes, during
  // render rather than in an effect (React's adjust-state-on-prop-change
  // pattern), so the previous member's profile never paints under the new id.
  const [renderedUserId, setRenderedUserId] = useState(userId);
  if (userId !== renderedUserId) {
    setRenderedUserId(userId);
    if (userId) {
      setLoading(true);
      setError(false);
      setInfo(null);
      setCounts({ paints: 0, badges: 0, sn: 0 });
      setNamePaint(null);
      setActivePaint(null);
      setSeventvUserId(null);
      setTheme(null);
      setHiddenSections([]);
      setWornBadges({ seventv: null, twitch: null, thirdParty: [], bttvPro: null });
    }
  }

  useEffect(() => {
    if (!userId) return;
    let alive = true;

    // What the live sources have already set. The first paint can land after
    // them (the server rebuilds a card in the background and a change lags by up
    // to a minute), so it fills only what nothing live has filled yet. A flag is
    // set only by a read that answered.
    const live = noLive();

    (async () => {
      try {
        // Instant paint: one read in Rust of the card streamnook.app wrote, or
        // just the member's identity when there is none. Both are already
        // checked there; this only applies them.
        const snapPromise = invoke<FirstPaint | null>('get_profile_snapshot', { userId }).catch(() => null);
        void (async () => {
          const snap = await snapPromise;
          if (!snap || !alive) return;
          // In live-preview mode the badge row is owned by the override path
          // (re-resolved on each loadout edit); a late card must not clobber a
          // fresher preview resolve.
          const plan = planFirstPaint(snap, live, followsLivePreview && !!useAppStore.getState().profileViewerPreview);
          if (plan.info) setInfo(plan.info);
          const cardCounts = plan.counts;
          if (cardCounts) setCounts((prev) => ({ ...prev, ...cardCounts }));
          if (plan.namePaint !== undefined) setNamePaint(plan.namePaint);
          if (plan.hiddenSections) setHiddenSections(plan.hiddenSections);
          const cardBadges = plan.wornBadges;
          if (cardBadges) setWornBadges((prev) => ({ ...prev, ...cardBadges }));
          if (plan.theme) {
            await whenAtmospheresReady();
            if (alive && !live.theme) setTheme(resolveProfileTheme(plan.theme.profileTheme, plan.theme.namePaint));
          }
          if (alive && plan.endLoading) setLoading(false);
        })();

        // Kick off the profile-theme prefs IMMEDIATELY (only needs userId) so the
        // backdrop resolves in parallel with the cosmetic + badge fetches below.
        // An Atmosphere needs only these prefs + the startup-loaded catalog, so it
        // paints right away instead of waiting on the whole cosmetic chain (which
        // was making the backdrop appear seconds late). Paint themes still finish
        // in the main chain since they need the 7TV paint data.
        // A read that failed (`ok` false, or a throw) is null here: it sets
        // nothing and marks nothing live, so the first-paint card still stands.
        const prefsPromise = getProfilePrefs(userId).catch(() => null);
        void (async () => {
          const plan = planPrefs(await prefsPromise);
          if (!alive) return;
          Object.assign(live, plan.live);
          if (plan.hiddenSections) setHiddenSections(plan.hiddenSections);
          if (plan.theme) {
            await whenAtmospheresReady();
            if (alive) setTheme(resolveProfileTheme(plan.theme.profileTheme, null));
          }
        })();

        // Resolve the viewed user's login + avatar + name from their id (the
        // badge only carries the id). One Helix lookup with the viewer's creds.
        const data = await helixGet<{ data?: Array<{ login?: string; display_name?: string; profile_image_url?: string }> }>('users', `id=${encodeURIComponent(userId)}`).catch(() => null);
        const u = data?.data?.[0];
        if (!u?.login) {
          if (alive) { setError(true); setLoading(false); }
          return;
        }
        if (!alive) return;
        live.info = true;
        setInfo({ login: u.login, displayName: u.display_name || u.login, avatar: u.profile_image_url || '' });
        setLoading(false);

        // Cosmetic counts + optional 7TV paint theme (best effort).
        try {
          const profile = await getFullProfileWithFallback(userId, u.login, userId, u.login);
          // Whether 7TV genuinely answered, decided when this profile was fetched
          // and kept on it: the inventory for the counts, the worn read for the
          // paint and badge. A failed read must not blank what the first-paint
          // card drew, nor mark it live.
          const paintAnswered = profile.wornAnswered === true;
          const countsStep = sevenTvCountsStep({
            answered: profile.inventoryAnswered === true,
            // An unloaded registry answers 0 for everyone: not an answer.
            snAnswered: isCosmeticsRegistryLoaded(),
            counts: {
              paints: profile.seventvCosmetics?.paints?.length ?? 0,
              badges: profile.seventvCosmetics?.badges?.length ?? 0,
              sn: getOwnedCosmeticSlugs(userId).size,
            },
          });
          if (alive) {
            Object.assign(live, countsStep.marks);
            setCounts(countsStep.apply);
          }

          // Worn identity badges, resolved the same way chat does (see
          // resolveWornBadges). Destructured so the snapshot build below can reuse
          // the resolved values; the live-preview effect re-runs the same helper.
          const { seventv: sevenTvBadge, twitch, thirdParty, bttvPro, answers } =
            await resolveWornBadges(userId, profile);
          const badgesStep = sevenTvBadgesStep<typeof wornBadges>({
            answered: paintAnswered,
            others: answers,
            row: { seventv: sevenTvBadge, twitch, thirdParty, bttvPro },
          });
          if (alive) {
            Object.assign(live, badgesStep.marks);
            setWornBadges(badgesStep.apply);
          }

          // The member's active 7TV paint always paints their NAME (free, like
          // chat). The profile BACKGROUND theme is premium: their 7TV paint or a
          // StreamNook Atmosphere. Free / non-premium falls back to the tier aura.
          const paints = profile.seventvCosmetics?.paints as Array<{ selected?: boolean }> | undefined;
          const activePaint = paints?.find((p) => p?.selected);
          const ps = activePaint ? computePaintStyle(activePaint as never, '#9146FF') : null;
          if (alive) {
            const namePaintPlan = planNamePaint({ answered: paintAnswered, paintStyle: ps });
            if (namePaintPlan.set) {
              live.paint = true;
              setNamePaint(namePaintPlan.value);
            }
            setActivePaint((activePaint as ({ id: string; name: string } & Record<string, unknown>) | undefined) ?? null);
            setSeventvUserId(profile.seventvCosmetics?.seventvUserId ?? null);
          }

          // The profile-theme prefs were kicked off at the TOP of this block (in
          // parallel with the cosmetic/badge fetches), and hiddenSections + any
          // Atmosphere/tier backdrop are already applied. Here we only finish the
          // PAINT theme — the one case that needs the 7TV paint data (`ps`).
          // With no paint it clears the theme, so a stale card cannot keep one, but
          // only when 7TV answered: after a failed fetch the card's paint aura stays.
          const paintTheme = planPaintTheme(await prefsPromise, { answered: paintAnswered, hasPaint: !!ps });
          if (paintTheme && alive) {
            Object.assign(live, paintTheme.live);
            setTheme(paintTheme.theme ? resolveProfileTheme('paint', ps) : null);
          }
        } catch { /* counts + theme stay at defaults */ }
      } catch (e) {
        Logger.error('[ProfileViewer] failed to load:', e);
        if (alive) { setError(true); setLoading(false); }
      }
    })();

    return () => { alive = false; };
  }, [userId, followsLivePreview]);

  // Live preview only: when the editor bumps badgeRevision (a loadout edit),
  // re-resolve the worn-badge row so it matches what others will see. The editor
  // calls setIdentity BEFORE bumping, so the identity caches are already primed
  // (fresh loadout keys) and cleared (third-party re-fetch). Uses the same helper
  // as the initial resolve, so the result is a true 1:1. Revision 0 is the first
  // open, already resolved by the main effect above.
  useEffect(() => {
    if (!userId || !preview || preview.badgeRevision === 0) return;
    let alive = true;
    (async () => {
      const login = info?.login ?? '';
      const profile = await getFullProfileWithFallback(userId, login, userId, login).catch(() => null);
      if (!profile || !alive) return;
      const { seventv, twitch, thirdParty, bttvPro } = await resolveWornBadges(userId, profile);
      const row = { seventv, twitch, thirdParty, bttvPro };
      // The preview exists to show the member's own loadout edit, so the badges
      // the edit picks always apply; only a failed 7TV read keeps the 7TV badge
      // the preview already shows.
      const step = sevenTvBadgesStep<typeof wornBadges>({
        answered: profile.wornAnswered === true,
        others: PREVIEW_OTHERS,
        row,
      });
      if (alive) setWornBadges(step.apply);
    })();
    return () => { alive = false; };
    // info is read at call time on purpose; the only trigger is a revision bump.
  }, [userId, preview?.badgeRevision]);

  const memberNumber = userId ? getStreamNookUserNumber(userId) : null;
  const cosmeticName = getWornBadgeCaption(userId);

  // The overlay background is the member's premium theme (7TV paint or a
  // StreamNook Atmosphere) when set, else the free tier aura. The premium theme
  // also drives the border/accent color; the painted name is free regardless.
  const tierAccent = memberNumber !== null ? getTierAccent(memberNumber) : null;
  // In live-preview mode the override is authoritative for the hidden sections
  // and theme, regardless of what the async loaders set in local state, so a
  // Settings edit reflects instantly and a late fetch can't clobber it.
  // getAtmosphere is a sync registry read (catalog loaded at startup), so
  // resolving the previewed theme here is safe.
  const effectiveHiddenSections = preview ? preview.hiddenSections : hiddenSections;
  const effectiveTheme = preview ? resolveProfileTheme(preview.profileTheme, namePaint) : theme;
  const themeRgb = effectiveTheme?.accentRgb ?? tierAccent?.rgb ?? null;
  const panelBorder = themeRgb ? `rgba(${themeRgb}, 0.22)` : undefined;
  const headerBorder = themeRgb ? `rgba(${themeRgb}, 0.14)` : undefined;
  const tierAura = tierAccent
    ? `radial-gradient(ellipse 90% 55% at 50% 0%, rgba(${tierAccent.rgb}, ${tierAccent.auraAlpha}), transparent 70%)`
    : undefined;

  // The Atmosphere (subscriber tier) is the full "whole profile" theme: it fills
  // the entire overlay and is softened to an ambient wash behind the content. A
  // 7TV paint theme (supporter tier) is deliberately LESS than that: just a
  // subtle accent in the hero, never bleeding through the content body, so the
  // two tiers read clearly different (paint < atmosphere).
  const hasAtmosphere = !!effectiveTheme?.atmosphere;

  return {
    info,
    counts,
    loading,
    error,
    namePaint,
    activePaint,
    seventvUserId,
    wornBadges,
    profileViews,
    memberNumber,
    cosmeticName,
    tierAccent,
    effectiveHiddenSections,
    effectiveTheme,
    themeRgb,
    panelBorder,
    headerBorder,
    tierAura,
    hasAtmosphere,
  };
}

export type MemberProfileView = ReturnType<typeof useMemberProfile>;
