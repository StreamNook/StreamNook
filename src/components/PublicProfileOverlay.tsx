import { useEffect, useState, useSyncExternalStore, type CSSProperties } from 'react';
import { motion, AnimatePresence, useDragControls } from 'framer-motion';
import { X, User, Eye } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { helixGet } from '../services/helix';
import { useAppStore } from '../stores/AppStore';
import { Logger } from '../utils/logger';
import ProfileOverview from './settings/ProfileOverview';
import { RelicStrip } from './profile/RelicStrip';
import { ProfileFrame } from './profile/ProfileFrame';
import { StreamNookBadge, getTierAccent, getTier } from './StreamNookBadge';
import { ProfileAccentContext, ProfileCompactContext } from './settings/profileAccentContext';
import { getFullProfileWithFallback } from '../services/cosmeticsCache';
import { computePaintStyle, getBadgeImageUrls, getBadgeFallbackUrls } from '../services/seventvService';
import { getAtmosphere, type Atmosphere } from '../services/atmospheres';
import { AtmosphereBackground } from './AtmosphereBackground';
import { getResolvedIdentity, getIdentityWithCache } from '../services/identityService';
import { resolveBttvProUrl, BTTV_PRO_LOADOUT_KEY } from '../services/bttvProBadge';
import { FallbackImage } from './FallbackImage';
import { Tooltip } from './ui/Tooltip';
import {
  getStreamNookUserNumber,
  getOwnedCosmeticSlugs,
  getActiveCosmeticSlug,
  getCosmeticBySlug,
  getProfilePrefs,
  subscribeStreamNookRegistryVersion,
  getStreamNookRegistryVersion,
  subscribeCosmeticsVersion,
  getCosmeticsVersion,
  whenAtmospheresReady,
  getProfileSnapshot,
  upsertProfileSnapshot,
  getUserIdentity,
  incrementProfileView,
  getProfileViews,
  type ProfileSnapshot,
} from '../services/supabaseService';

// "#rrggbb" -> "r, g, b" for use inside rgba(...).
const hexToRgb = (hex: string): string | null => {
  const m = /^#?([0-9a-fA-F]{6})$/.exec(hex.trim());
  if (!m) return null;
  const n = parseInt(m[1], 16);
  return `${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}`;
};

// Resolve a global Twitch badge "set/version" key to an image. Global badges are
// universal, so this resolves for any member regardless of the viewer. Warms the
// Rust global-badge cache if it's cold.
const resolveGlobalTwitchBadge = async (
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

interface TargetInfo {
  login: string;
  displayName: string;
  avatar: string;
}

type ResolvedTheme = { accentRgb: string | null; paintAura?: CSSProperties; atmosphere?: Atmosphere };

// Build the resolved background theme from a member's profile_theme id + their
// resolved 7TV paint style (ps). Shared by the live resolve AND the snapshot
// hydrate so both paint identically (no flicker when the revalidate lands).
// Assumes the atmosphere catalog is loaded (getAtmosphere is a sync registry read).
const resolveProfileTheme = (
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

type WornBadges = {
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
const resolveWornBadges = async (userId: string, profile: any): Promise<WornBadges> => {
  const seventv =
    (profile?.seventvCosmetics?.badges as any[] | undefined)?.find((b) => b?.selected) ?? null;
  let twitch: { src: string; title: string } | null = null;
  let thirdParty: any[] = [];
  let bttvPro: { src: string; title: string } | null = null;
  try {
    const [resolved, rawLoadout] = await Promise.all([
      getResolvedIdentity(userId).catch(() => null),
      getIdentityWithCache(userId).catch(() => null),
    ]);
    thirdParty = (resolved?.badges ?? [])
      .filter((b: any) => b.provider !== 'twitch')
      .map((b: any) => ({ key: b.key, provider: b.provider, title: b.title, src: b.image_url }));
    const keys: string[] = rawLoadout?.badges ?? [];
    const twitchKey = keys.find((k) => k.startsWith('twitch:'));
    if (twitchKey) {
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
  } catch { /* badges optional */ }
  return { seventv, twitch, thirdParty, bttvPro };
};

const PublicProfileOverlay = () => {
  const userId = useAppStore((s) => s.profileViewerUserId);
  // When set, this is a LIVE preview of the current user's own profile: its
  // values override the fetched ones so Settings edits reflect instantly.
  const preview = useAppStore((s) => s.profileViewerPreview);
  const close = useAppStore((s) => s.closeProfileViewer);
  // The VIEWER's own id, so we don't count self-views (or the live preview).
  const currentUserId = useAppStore((s) => s.currentUser?.user_id);
  const dragControls = useDragControls();

  const [info, setInfo] = useState<TargetInfo | null>(null);
  const [counts, setCounts] = useState({ paints: 0, badges: 0, sn: 0 });
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(false);
  // The viewed member's active 7TV paint paints their NAME (free, like chat).
  const [namePaint, setNamePaint] = useState<CSSProperties | null>(null);
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
      setTheme(null);
      setHiddenSections([]);
      setWornBadges({ seventv: null, twitch: null, thirdParty: [], bttvPro: null });
    }
  }

  useEffect(() => {
    if (!userId) return;
    let alive = true;

    (async () => {
      try {
        // Instant paint from the cached snapshot (if any): hydrate everything the
        // overlay renders from ONE read, before the live sources resolve. The live
        // chain below revalidates and rewrites the cache (stale-while-revalidate).
        const snapPromise = getProfileSnapshot(userId);
        void (async () => {
          const s = await snapPromise;
          if (!s || !alive) return;
          const snap = s.snapshot;
          setInfo(snap.identity);
          setCounts(snap.counts);
          setNamePaint((snap.namePaint as CSSProperties) ?? null);
          setHiddenSections(snap.hiddenSections);
          // In live-preview mode the badge row is owned by the override path
          // (re-resolved on each loadout edit); skip the snapshot so a late
          // cache read can't clobber a fresher preview resolve.
          if (!useAppStore.getState().profileViewerPreview) {
            setWornBadges({
              seventv: snap.seventvBadge,
              twitch: snap.wornBadges.twitch,
              thirdParty: snap.wornBadges.thirdParty,
              bttvPro: snap.wornBadges.bttvPro,
            });
          }
          await whenAtmospheresReady();
          if (alive) {
            setTheme(resolveProfileTheme(snap.profileTheme, (snap.namePaint as CSSProperties) ?? null));
            setLoading(false);
          }
        })();
        // No snapshot yet → read our own `users` table for instant identity
        // (faster than the Twitch Helix call below, which then revalidates it).
        void (async () => {
          const s = await snapPromise;
          if (s || !alive) return;
          const id = await getUserIdentity(userId);
          if (id && alive) {
            setInfo(id);
            setLoading(false);
          }
        })();

        // Kick off the profile-theme prefs IMMEDIATELY (only needs userId) so the
        // backdrop resolves in parallel with the cosmetic + badge fetches below.
        // An Atmosphere needs only these prefs + the startup-loaded catalog, so it
        // paints right away instead of waiting on the whole cosmetic chain (which
        // was making the backdrop appear seconds late). Paint themes still finish
        // in the main chain since they need the 7TV paint data.
        const prefsPromise = getProfilePrefs(userId).catch(
          () => ({ profileTheme: 'tier', hiddenSections: [] as string[] }),
        );
        void (async () => {
          const prefs = await prefsPromise;
          if (!alive) return;
          setHiddenSections(prefs.hiddenSections ?? []);
          if (prefs.profileTheme !== 'paint') {
            await whenAtmospheresReady();
            if (alive) setTheme(resolveProfileTheme(prefs.profileTheme, null));
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
        setInfo({ login: u.login, displayName: u.display_name || u.login, avatar: u.profile_image_url || '' });
        setLoading(false);

        // Cosmetic counts + optional 7TV paint theme (best effort).
        try {
          const profile = await getFullProfileWithFallback(userId, u.login, userId, u.login);
          if (alive) {
            setCounts({
              paints: profile.seventvCosmetics?.paints?.length ?? 0,
              badges: profile.seventvCosmetics?.badges?.length ?? 0,
              sn: getOwnedCosmeticSlugs(userId).size,
            });
          }

          // Worn identity badges, resolved the same way chat does (see
          // resolveWornBadges). Destructured so the snapshot build below can reuse
          // the resolved values; the live-preview effect re-runs the same helper.
          const { seventv: sevenTvBadge, twitch, thirdParty, bttvPro } =
            await resolveWornBadges(userId, profile);
          if (alive) setWornBadges({ seventv: sevenTvBadge, twitch, thirdParty, bttvPro });

          // The member's active 7TV paint always paints their NAME (free, like
          // chat). The profile BACKGROUND theme is premium: their 7TV paint or a
          // StreamNook Atmosphere. Free / non-premium falls back to the tier aura.
          const paints = profile.seventvCosmetics?.paints as Array<{ selected?: boolean }> | undefined;
          const activePaint = paints?.find((p) => p?.selected);
          const ps = activePaint ? computePaintStyle(activePaint as never, '#9146FF') : null;
          if (alive) setNamePaint(ps);

          // The profile-theme prefs were kicked off at the TOP of this block (in
          // parallel with the cosmetic/badge fetches), and hiddenSections + any
          // Atmosphere/tier backdrop are already applied. Here we only finish the
          // PAINT theme — the one case that needs the 7TV paint data (`ps`).
          const prefs = await prefsPromise;
          if (prefs.profileTheme === 'paint' && ps && alive) {
            setTheme(resolveProfileTheme('paint', ps));
          }

          // Refresh the cache (cache-aside) from the freshly-resolved values, so the
          // next open of this member (by anyone) paints instantly. Throttled: only
          // write when the snapshot is missing, older than 60s, or a key field
          // changed — so popular profiles aren't rewritten on every view.
          const existing = await snapPromise;
          const freshSnap: ProfileSnapshot = {
            v: 1,
            identity: {
              login: u.login,
              displayName: u.display_name || u.login,
              avatar: u.profile_image_url || '',
            },
            profileTheme: prefs.profileTheme,
            hiddenSections: prefs.hiddenSections ?? [],
            memberNumber: getStreamNookUserNumber(userId) ?? null,
            cosmeticSlug: getActiveCosmeticSlug(userId) ?? null,
            namePaint: ps ? (ps as Record<string, unknown>) : null,
            seventvBadge: (sevenTvBadge as Record<string, unknown> | null) ?? null,
            wornBadges: { twitch, thirdParty, bttvPro },
            counts: {
              paints: profile.seventvCosmetics?.paints?.length ?? 0,
              badges: profile.seventvCosmetics?.badges?.length ?? 0,
              sn: getOwnedCosmeticSlugs(userId).size,
            },
            stats: null,
            accolades: [],
            favoriteChannel: null,
            ivr: null,
          };
          const stale =
            !existing ||
            Date.now() - new Date(existing.updatedAt).getTime() > 60_000 ||
            existing.snapshot.profileTheme !== freshSnap.profileTheme ||
            existing.snapshot.cosmeticSlug !== freshSnap.cosmeticSlug ||
            existing.snapshot.identity.avatar !== freshSnap.identity.avatar;
          if (stale) void upsertProfileSnapshot(userId, freshSnap);
        } catch { /* counts + theme stay at defaults */ }
      } catch (e) {
        Logger.error('[ProfileViewer] failed to load:', e);
        if (alive) { setError(true); setLoading(false); }
      }
    })();

    return () => { alive = false; };
  }, [userId]);

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
      const wb = await resolveWornBadges(userId, profile);
      if (alive) setWornBadges(wb);
    })();
    return () => { alive = false; };
    // info is read at call time on purpose; the only trigger is a revision bump.
  }, [userId, preview?.badgeRevision]);

  const memberNumber = userId ? getStreamNookUserNumber(userId) : null;
  const cosmeticSlug = userId ? getActiveCosmeticSlug(userId) : null;
  const cosmeticName = cosmeticSlug ? getCosmeticBySlug(cosmeticSlug)?.name ?? null : null;

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
  const panelStyle = { borderColor: panelBorder } as CSSProperties & Record<string, string>;

  return (
    <AnimatePresence>
      {userId && (
        <motion.div
          key="profile-viewer"
          drag
          dragControls={dragControls}
          dragListener={false}
          dragMomentum={false}
          initial={{ opacity: 0, scale: 0.97 }}
          animate={{ opacity: 1, scale: 1 }}
          exit={{ opacity: 0, scale: 0.97 }}
          transition={{ type: 'spring', stiffness: 380, damping: 30 }}
          className="sn-light-off fixed left-[calc(50%-380px)] top-12 z-[60] flex max-h-[86vh] w-[760px] max-w-[94vw] flex-col overflow-hidden rounded-xl border border-white/10 bg-[rgba(14,14,18,0.96)] shadow-[0_24px_60px_-15px_rgba(0,0,0,0.8)] backdrop-blur-2xl"
          style={panelStyle}
        >
          {/* Whole-overlay backdrop. Only the Atmosphere (subscriber tier) fills
              the ENTIRE profile (it IS the vibe), softened to an ambient wash
              behind the content. A 7TV paint theme does NOT go here — it's a
              hero-only accent (below), so it stays clearly below the atmospheres.
              Everything else gets the subtle tier radial (incl. behind a paint
              theme). */}
          {hasAtmosphere ? (
            <AtmosphereBackground
              atm={effectiveTheme!.atmosphere!}
              variant="profile"
              blur={!!effectiveTheme!.atmosphere!.image}
            />
          ) : tierAura ? (
            <motion.div
              className="pointer-events-none absolute -inset-10"
              style={{ background: tierAura }}
              animate={{ opacity: [0.7, 1, 0.7] }}
              transition={{ duration: 7, repeat: Infinity, ease: 'easeInOut' }}
            />
          ) : null}

          {/* Hero band — identity + rank over the atmosphere (most present here).
              Doubles as the drag handle. */}
          <div
            onPointerDown={(e) => dragControls.start(e)}
            style={{ borderBottomColor: headerBorder }}
            className="relative z-[2] flex-shrink-0 cursor-grab border-b border-white/[0.06] shadow-[0_12px_24px_-12px_rgba(0,0,0,0.85)] active:cursor-grabbing"
          >
            {/* Readability scrim so the identity reads over a busy backdrop. */}
            <div
              className="pointer-events-none absolute inset-0 z-[1]"
              style={{
                background:
                  'linear-gradient(180deg, rgba(10,10,14,0.30) 0%, rgba(10,10,14,0.55) 55%, rgba(10,10,14,0.82) 100%)',
              }}
            />
            {/* 7TV paint accent (supporter tier): a subtle blurred wash of the
                member's paint at the TOP of the hero only, fading out. Deliberately
                restrained vs a full Atmosphere so the paint theme reads as the
                lesser tier. */}
            {effectiveTheme?.paintAura && (
              <motion.div
                className="pointer-events-none absolute inset-x-0 top-0 z-[1] h-full"
                style={{
                  ...effectiveTheme.paintAura,
                  WebkitMaskImage: 'linear-gradient(to bottom, black, transparent 80%)',
                  maskImage: 'linear-gradient(to bottom, black, transparent 80%)',
                }}
                animate={{ opacity: [0.16, 0.24, 0.16] }}
                transition={{ duration: 7, repeat: Infinity, ease: 'easeInOut' }}
              />
            )}
            {/* Top + bottom specular hairlines (catch light, seam into content). */}
            <div className="pointer-events-none absolute inset-x-0 top-0 z-[2] h-px bg-gradient-to-r from-transparent via-white/20 to-transparent" />
            <div className="pointer-events-none absolute inset-x-0 bottom-0 z-[2] h-px bg-gradient-to-r from-transparent via-white/[0.12] to-transparent" />

            {/* The member's equipped Frame, bordering the hero band. */}
            <ProfileFrame userId={userId} />
            {/* Identity — seated and vertically centered in the hero. */}
            <div className="relative z-10 flex items-center gap-3 p-4">
              <div className="flex min-w-0 flex-1 items-center gap-3.5">
                <span className="flex h-16 w-16 flex-shrink-0 items-center justify-center overflow-hidden rounded-full bg-white/[0.04] shadow-[inset_0_1px_0_0_rgba(255,255,255,0.18),0_4px_12px_rgba(0,0,0,0.5)] ring-1 ring-inset ring-white/15">
                  {info?.avatar ? (
                    <img src={info.avatar} alt="" className="h-full w-full object-cover" draggable={false} />
                  ) : (
                    <User size={28} className="text-textSecondary" />
                  )}
                </span>
                <div className="min-w-0 flex-1">
                  {/* Name, with the Preview pill inline. */}
                  <div className="flex items-center gap-2">
                    <span
                      className="truncate text-xl font-bold leading-tight text-textPrimary"
                      style={namePaint ?? undefined}
                    >
                      {info?.displayName ?? 'StreamNook member'}
                    </span>
                    {preview && (
                      <span className="flex-shrink-0 rounded-full border border-accent/30 bg-accent/15 px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wide text-accent">
                        Preview
                      </span>
                    )}
                  </div>
                  {/* Meta line: @handle + the worn badges grouped together, so the
                      badges read as part of the identity instead of a stray row
                      dangling under the name. */}
                  <div className="mt-1.5 flex flex-wrap items-center gap-x-2.5 gap-y-1.5">
                    {info?.login && (
                      <span className="text-[13px] leading-none text-textMuted">@{info.login}</span>
                    )}
                    {(() => {
                      const { seventv, twitch, thirdParty, bttvPro } = wornBadges;
                      const hasAny =
                        !!twitch || !!seventv || memberNumber !== null || thirdParty.length > 0 || !!bttvPro;
                      if (!hasAny) return null;
                      const imgBadge = (src: string, title: string, key: string) => (
                        <Tooltip key={key} content={title} side="bottom">
                          <img
                            src={src}
                            alt={title}
                            draggable={false}
                            className="h-[18px] w-[18px] flex-shrink-0 object-contain"
                          />
                        </Tooltip>
                      );
                      // stopPropagation so a badge click opens the nested profile
                      // instead of starting a drag.
                      return (
                        // Canonical badge order (see utils/badgeOrder): StreamNook
                        // leads, then Twitch global, then 7TV, then third-party
                        // (BTTV Pro among them). No channel context here, so the
                        // channel-contextual tier (sub/poll) never appears.
                        <div
                          className="flex flex-wrap items-center gap-1.5"
                          onPointerDown={(e) => e.stopPropagation()}
                        >
                          {memberNumber !== null && userId && (
                            <StreamNookBadge userId={userId} userNumber={memberNumber} side="bottom" />
                          )}
                          {twitch && imgBadge(twitch.src, `Twitch: ${twitch.title}`, 'tw')}
                          {seventv && (() => {
                            const urls = getBadgeImageUrls(seventv as any);
                            return urls.url4x ? (
                              <Tooltip content={`7TV: ${seventv.tooltip || seventv.name}`} side="bottom">
                                <FallbackImage
                                  src={urls.url4x}
                                  fallbackUrls={getBadgeFallbackUrls(seventv.id).slice(1)}
                                  alt={seventv.tooltip || seventv.name}
                                  className="h-[18px] w-[18px] flex-shrink-0"
                                />
                              </Tooltip>
                            ) : null;
                          })()}
                          {thirdParty.map((b: any) =>
                            imgBadge(b.src, `${b.title} (${b.provider.toUpperCase()})`, b.key || b.title),
                          )}
                          {bttvPro && imgBadge(bttvPro.src, 'BTTV Pro', 'bttvpro')}
                        </div>
                      );
                    })()}
                    {/* Applied atmosphere: a labeled chip (swatch + name) so a
                        viewer can see WHICH atmosphere the member is wearing, the
                        way the worn 7TV paint is surfaced. The animated wash
                        itself is already the hero backdrop above. */}
                    {effectiveTheme?.atmosphere && (
                      <Tooltip content="Applied atmosphere" side="bottom">
                        <span className="flex items-center gap-1.5 rounded-full bg-white/[0.05] px-2 py-0.5 text-[11px] leading-none text-textSecondary ring-1 ring-inset ring-white/10">
                          <span
                            className="h-3 w-3 flex-shrink-0 rounded-full ring-1 ring-inset ring-white/20"
                            style={{ background: effectiveTheme.atmosphere.swatch }}
                          />
                          {effectiveTheme.atmosphere.name}
                        </span>
                      </Tooltip>
                    )}
                    {/* Profile views — a subtle public counter. Hideable via the
                        'views' visibility toggle (honored here so the live preview
                        reflects what others see). */}
                    {profileViews != null && !effectiveHiddenSections.includes('views') && (
                      <Tooltip content="Profile views" side="bottom">
                        <span className="flex items-center gap-1 text-[11px] leading-none text-textMuted">
                          <Eye size={13} className="opacity-80" />
                          {profileViews.toLocaleString()}
                        </span>
                      </Tooltip>
                    )}
                  </div>
                </div>
              </div>

              {/* Rank identity, lifted from the tier card but WITHOUT its chassis
                  so it blends into the hero instead of reading as a card on top.
                  The decode cypher still lives on the badge hover. */}
              {memberNumber !== null && (() => {
                const tier = getTier(memberNumber);
                return (
                  <div className="flex flex-shrink-0 flex-col items-end pt-0.5 text-right">
                    <div className="flex items-baseline gap-1.5">
                      <span className="text-[11px] font-light leading-none text-white/35">Nº</span>
                      <span className={tier.numberClassName}>{memberNumber.toLocaleString()}</span>
                    </div>
                    <div className={`mb-2 mt-2.5 h-px w-12 ${tier.hairlineClassName}`} />
                    {tier.label && <div className={tier.labelClassName}>{tier.label}</div>}
                    {cosmeticName && (
                      <div className="mt-2.5 text-[10px] font-light uppercase tracking-[0.22em] text-white/70">
                        {cosmeticName}
                      </div>
                    )}
                  </div>
                );
              })()}

              {/* Close — inline at the top-right so it can't collide with the
                  rank number. stopPropagation so it never starts a drag. */}
              <button
                onClick={close}
                aria-label="Close"
                onPointerDown={(e) => e.stopPropagation()}
                className="-mr-1 -mt-1 flex-shrink-0 self-start rounded p-1.5 text-textMuted transition-colors hover:bg-white/[0.10] hover:text-textPrimary"
              >
                <X size={16} />
              </button>
            </div>
          </div>

          {/* Content region. The atmosphere continues behind here (so it themes
              the WHOLE profile) but is softened to an ambient wash by a heavy
              blur + dim cover, so it's felt without competing with the stats.
              The panels stay semi-translucent so a hint of the wash tints them. */}
          <div className="relative z-[1] flex min-h-0 flex-1 flex-col">
            {hasAtmosphere && (
              <div className="pointer-events-none absolute inset-0 bg-[rgba(10,10,14,0.58)] backdrop-blur-[44px]" />
            )}
            {/* Scroll body. Bounds its height via flex (flex-1 + min-h-0) rather
                than height:100% — percentage-height resolution against a flex
                item is unreliable across WebView2 runtime versions, which left
                the body unbounded and unscrollable on some installs. The dim
                cover above stays a non-scrolling backdrop (absolute, out of flow). */}
            <div
              className="scrollbar-thin relative z-[1] min-h-0 flex-1 overflow-y-auto p-3"
              style={hasAtmosphere ? ({ '--glass-strength': '0.45' } as CSSProperties & Record<string, string>) : undefined}
            >
            {loading ? (
              <div className="flex h-32 items-center justify-center">
                <span className="h-5 w-5 animate-spin rounded-full border-2 border-white/20 border-t-white/70" />
              </div>
            ) : error || !info ? (
              <p className="py-8 text-center text-sm text-textSecondary">Couldn't load this profile.</p>
            ) : (
              <ProfileCompactContext.Provider value={true}>
              <ProfileAccentContext.Provider value={themeRgb}>
                <RelicStrip userId={userId} />
                <ProfileOverview
                  isOwnProfile={false}
                  userId={userId}
                  login={info.login}
                  displayName={info.displayName}
                  broadcasterType=""
                  streamNookUserNumber={memberNumber}
                  seventvPaintCount={counts.paints}
                  seventvBadgeCount={counts.badges}
                  ownedCosmeticsCount={counts.sn}
                  hiddenSections={effectiveHiddenSections}
                />
              </ProfileAccentContext.Provider>
              </ProfileCompactContext.Provider>
            )}
            </div>
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
};

export default PublicProfileOverlay;
