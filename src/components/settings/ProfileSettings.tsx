import { useState, useEffect, useRef, useSyncExternalStore, useMemo, type CSSProperties } from 'react';
import { createPortal } from 'react-dom';
import { AnimatePresence, motion } from 'framer-motion';
import { useAppStore } from '../../stores/AppStore';
import { refreshAtmosphere } from '../../stores/chatUserStore';
import { AtmosphereBackground } from '../AtmosphereBackground';
import { isDevAtmosphere } from '../../services/devAtmospheres';
import { MajorCologneChrome } from '../MajorCologneChrome';
import { getPreviewEmotes, previewEmoteUrl, rollPreviewChat, type PreviewEmote } from '../../utils/previewChat';
import { openBadgesWithPaintInMain, openBadgesOnStreamNookInMain } from '../../utils/openBadgesInMain';
import streamNookLogo from '../../assets/streamnook-logo-128.webp';
import { User, Link, Unlink, Image as ImageIcon, Film, Heart, Check, ExternalLink, Lock } from 'lucide-react';
import {
  computePaintStyle,
  getBadgeImageUrls,
  getBadgeFallbackUrls,
  queueCosmeticForCaching,
  clearUserCache as clear7TVCache,
} from '../../services/seventvService';
import { FallbackImage } from '../FallbackImage';
import { Tooltip } from '../ui/Tooltip';
import { TwitchBadge } from '../../services/badgeService';
import { ThirdPartyBadge } from '../../services/thirdPartyBadges';
import { SevenTVBadge, SevenTVPaint } from '../../types';
import {
  getProfileFromMemoryCache,
  getFullProfileWithFallback,
  CachedProfile,
} from '../../services/cosmeticsCache';
import {
  getStreamNookUserNumber,
  subscribeStreamNookRegistryVersion,
  getStreamNookRegistryVersion,
  subscribeCosmeticsVersion,
  getCosmeticsVersion,
  subscribeAtmospheresVersion,
  getAtmospheresVersion,
  getAllCosmetics,
  getOwnedCosmeticSlugs,
  getActiveCosmeticSlug,
  setActiveCosmetic,
  getProfilePrefs,
  setProfileTheme,
  patchProfileSnapshotTheme,
  setHiddenSections,
  getAccolades,
} from '../../services/supabaseService';
import type { CosmeticCatalogEntry } from '../../services/supabaseService';
import { getIdentityWithCache, setIdentity } from '../../services/identityService';
import { readOwnProfileCache, writeOwnProfileCache } from '../../services/ownProfileCache';
import { isSubscriber } from '../../services/subscriberService';
import { listAtmospheres, getAtmosphere, getAtmosphereUnlock, type Atmosphere } from '../../services/atmospheres';
import { resolveEntitlement } from '../../services/cosmetics/ownership';
import { MAJOR_COLOGNE_THEME_ID, MAJOR_COLOGNE_ACCOLADE_ID, parseCologneTheme, buildCologneTheme, isCologneTheme } from '../../services/cologneEvent';
import { getTier, getTierAccent, StreamNookBadge } from '../StreamNookBadge';
import { resolveCosmeticAsset } from '../cosmeticAssets';
import {
  captureProfileCard,
  copyImageToClipboard,
  downloadBlob,
  estimateCaptureDurationMs,
  detectAnimatedPaint,
  pauseCardAnimations,
  type CaptureMode,
} from '../../utils/shareProfile';
import { clearCosmeticsMemoryCache, invalidateUserCosmetics, getCosmeticsWithFallback, applyLocalCosmeticSelection } from '../../services/cosmeticsCache';
import { buildBttvProBadge, resolveBttvProUrl } from '../../services/bttvProBadge';
import { invoke } from '@tauri-apps/api/core';
import { Logger } from '../../utils/logger';
import LinkedAccountsSection from './LinkedAccountsSection';
import ProfileOverview from './ProfileOverview';

export interface ChatIdentityCache {
  badges: ChatIdentityBadge[];
  lastFetched: number;
  userId: string;
}

export let chatIdentityCache: ChatIdentityCache | null = null;
export const CHAT_IDENTITY_CACHE_TTL = 10 * 60 * 1000;
export const CHAT_IDENTITY_BACKGROUND_REFRESH_TTL = 2 * 60 * 1000;

export const setChatIdentityCache = (cache: ChatIdentityCache | null) => {
  chatIdentityCache = cache;
};

interface ChatIdentityBadge {
  id: string;
  version: string;
  title: string;
  image_url: string;
  is_selected: boolean;
}

// Twitch badge image URLs are universal (`/badges/v1/<imageId>/<scale>`), so the
// image id renders the same badge for any viewer. We store `twitch:<imageId>` in
// the loadout (not set/version, which only resolves GLOBAL badges from a warm
// cache) so other clients can render even channel-scoped badges cross-client.
const extractTwitchBadgeImageId = (imageUrl: string): string | null => {
  const m = /\/badges\/v1\/([^/]+)/.exec(imageUrl);
  return m ? m[1] : null;
};

// Public-profile sections the member can hide from other viewers. Most keys
// match ProfileOverview's `sectionHidden`; `views` is honored by the overlay's
// hero (the profile-view counter), not a ProfileOverview section. Subs/spend are
// never persisted, so they're not toggleable here.
const VISIBILITY_SECTIONS: Array<{ key: string; label: string }> = [
  { key: 'roast', label: 'Hours watched' },
  { key: 'twitch', label: 'Your Twitch (age, followers, type)' },
  { key: 'lifetime', label: 'Lifetime stats' },
  { key: 'emotes', label: 'Top emotes' },
  { key: 'accolades', label: 'Accolades' },
  { key: 'views', label: 'Profile views' },
];

// Selection indicator for the loadout pickers — a flat checkmark chip in the
// item's top-right corner (replaces the old status dot). `accent` for Twitch /
// StreamNook selections, the 7TV blue for 7TV cosmetics. Intentionally flat:
// border + solid fill, no glow or ring.
const CheckChip = ({ tone = 'accent' }: { tone?: 'accent' | 'seventv' }) => (
  <div
    className={`absolute -top-1.5 -right-1.5 w-4 h-4 rounded-full border border-background flex items-center justify-center ${
      tone === 'seventv' ? 'bg-[#29b6f6]' : 'bg-accent'
    }`}
  >
    <Check size={9} strokeWidth={3.5} className="text-white" />
  </div>
);

// The lock on a badge you have not earned yet. Amber like the tier chips on
// the theme rows, so "locked" reads the same way everywhere on this tab.
const LockChip = () => (
  <div className="absolute -top-1.5 -right-1.5 flex h-4 w-4 items-center justify-center rounded-full border border-background bg-[#2a2418]">
    <Lock size={8} strokeWidth={3} className="text-amber-300/90" />
  </div>
);

const ProfileSettings = () => {
  const { isAuthenticated, currentUser, loginToTwitch, currentStream, addToast, openProfilePreview, updateProfilePreview } = useAppStore();
  // Last-rendered loadout persisted from a prior open. Read ONCE, synchronously,
  // before the cosmetic state below is initialized, so the card paints
  // fully-dressed on the very first frame instead of flashing barebones while the
  // network catches up. The background fetches further down revalidate it.
  const [seededProfile] = useState(() => readOwnProfileCache(currentUser?.user_id));
  const profileCardRef = useRef<HTMLDivElement>(null);
  const [isSharing, setIsSharing] = useState(false);
  // Inner Profile sub-tab. Overview (stats showcase) is the default landing
  // view; Customize holds the identity editor that used to be the whole pane.
  const [profileTab, setProfileTab] = useState<'overview' | 'customize'>('overview');
  // Capture-progress UI state. `captureStage` drives which sub-component
  // shows: 'recording' = determinate bar tied to elapsed/estimated; one
  // 'finalizing' = indeterminate spinner once the wall-clock has caught
  // up to the source-animation duration (encode phase). null = idle.
  const [captureStage, setCaptureStage] = useState<'recording' | 'finalizing' | null>(
    null,
  );
  const [captureProgress, setCaptureProgress] = useState(0);
  // Tracks whether the rendered profile card contains anything that
  // would actually benefit from an animated capture (CSS animations on
  // the StreamNook tier glow, animated 7TV paints, animated badges or
  // emotes). Drives whether the "animated" share button is enabled.
  // Detection re-runs whenever the inputs that affect card rendering
  // change — see the useEffect further down.
  const [hasAnimatedElements, setHasAnimatedElements] = useState(false);
  // Portal target inside SettingsDialog's hero row (the right-side slot
  // alongside the "Profile" title). We render the share buttons into that
  // slot instead of inline above the card, so the controls sit at the
  // same vertical level as the hero text and don't push any layout down.
  // Looked up after mount because the dialog hero renders before this
  // child does — null until the effect runs.
  const [heroActionsTarget, setHeroActionsTarget] = useState<HTMLElement | null>(null);
  useEffect(() => {
    setHeroActionsTarget(document.getElementById('settings-hero-actions'));
  }, []);

  useSyncExternalStore(subscribeStreamNookRegistryVersion, getStreamNookRegistryVersion, getStreamNookRegistryVersion);
  const streamNookUserNumber = currentUser?.user_id ? getStreamNookUserNumber(currentUser.user_id) : null;

  useSyncExternalStore(subscribeCosmeticsVersion, getCosmeticsVersion, getCosmeticsVersion);
  useSyncExternalStore(subscribeAtmospheresVersion, getAtmospheresVersion, getAtmospheresVersion);
  const cosmeticsCatalog: CosmeticCatalogEntry[] = getAllCosmetics();
  const ownedCosmeticSlugs = currentUser?.user_id
    ? getOwnedCosmeticSlugs(currentUser.user_id)
    : new Set<string>();
  const ownedCosmetics = cosmeticsCatalog.filter((c) => ownedCosmeticSlugs.has(c.slug));
  // Only badges are equippable in the badge slot; relics/frames live in their own
  // slots, so keep non-badge kinds out of the badge picker below.
  const ownedBadges = ownedCosmetics.filter((c) => c.kind === 'badge');
  // Badges that exist but are not yours yet. Shown locked after your own, so
  // the whole collection is visible; staged/owner-only rows stay out of it.
  const lockedBadges = cosmeticsCatalog.filter(
    (c) => c.kind === 'badge' && c.is_active && !c.hidden && !ownedCosmeticSlugs.has(c.slug),
  );
  const activeCosmeticSlug = currentUser?.user_id
    ? getActiveCosmeticSlug(currentUser.user_id)
    : null;

  const [twitchBadges, setTwitchBadges] = useState<TwitchBadge[]>(
    () => (seededProfile?.twitchBadges as TwitchBadge[] | undefined) ?? [],
  );
  void twitchBadges;
  const [thirdPartyBadges, setThirdPartyBadges] = useState<ThirdPartyBadge[]>(
    () => (seededProfile?.thirdPartyBadges as ThirdPartyBadge[] | undefined) ?? [],
  );
  // Your own BTTV Pro badge, resolved client-side (it's WebSocket-only, so it
  // can't ride the server-resolved picker list). Null when you don't have Pro,
  // so the toggle only appears for Pro members.
  const [selfBttvProBadge, setSelfBttvProBadge] = useState<ReturnType<typeof buildBttvProBadge> | null>(
    () => (seededProfile?.selfBttvProBadge as ReturnType<typeof buildBttvProBadge> | undefined) ?? null,
  );
  const [seventvBadges, setSeventvBadges] = useState<SevenTVBadge[]>(
    () => (seededProfile?.seventvBadges as SevenTVBadge[] | undefined) ?? [],
  );
  const [seventvPaint, setSeventvPaint] = useState<SevenTVPaint | null>(
    () => (seededProfile?.seventvPaint as SevenTVPaint | null | undefined) ?? null,
  );
  const [profileTheme, setProfileThemeState] = useState(() => seededProfile?.profileTheme ?? 'tier');
  // The cosmetic currently hovered in the picker, for the live preview card.
  const [previewThemeId, setPreviewThemeId] = useState<string | null>(null);
  // Live 7TV global emotes for the mock chat message (fetched once, cached).
  const [previewEmotes, setPreviewEmotes] = useState<PreviewEmote[]>([]);
  useEffect(() => { getPreviewEmotes().then(setPreviewEmotes).catch(() => {}); }, []);
  // Sections the member has hidden from their public profile.
  const [hiddenSecs, setHiddenSecs] = useState<string[]>(() => seededProfile?.hiddenSections ?? []);
  const [loadoutLoaded, setLoadoutLoaded] = useState(false);
  const [subscribed, setSubscribed] = useState(false);
  // Accolades the member has earned (persisted in user_accolades). Used to
  // unlock achievement-gated Atmospheres like Midnight, which appear in the
  // picker only once earned (a free unlock, no subscription required).
  const [earnedAccolades, setEarnedAccolades] = useState<Set<string>>(new Set());
  const [allSeventvPaints, setAllSeventvPaints] = useState<SevenTVPaint[]>(
    () => (seededProfile?.allSeventvPaints as SevenTVPaint[] | undefined) ?? [],
  );
  const [seventvUserId, setSeventvUserId] = useState<string | null>(() => seededProfile?.seventvUserId ?? null);
  const [, setHas7TVAccountChecked] = useState(false);
  const [, setIsLoadingBadges] = useState(false);
  const [seventvAuthConnected, setSeventvAuthConnected] = useState(false);
  const [updatingSeventvPaintId, setUpdatingSeventvPaintId] = useState<string | null>(null);
  const [updatingSeventvBadgeId, setUpdatingSeventvBadgeId] = useState<string | null>(null);
  const [isConnecting7TV, setIsConnecting7TV] = useState(false);
  const [chatIdentityBadges, setChatIdentityBadges] = useState<ChatIdentityBadge[]>(
    () => (seededProfile?.chatIdentityBadges as ChatIdentityBadge[] | undefined) ?? [],
  );
  const [isFetchingIdentity, setIsFetchingIdentity] = useState(false);
  const [updatingBadgeId, setUpdatingBadgeId] = useState<string | null>(null);
  // Shared cross-client loadout: which third-party badges to promote into chat
  // and the profile card. `customized:false` ⇒ show everything (the default).
  const [loadout, setLoadout] = useState<{ customized: boolean; badges: string[] }>(
    () => seededProfile?.loadout ?? { customized: false, badges: [] },
  );

  // Mounted gate so async resolvers don't setState on a dead component when
  // the user navigates away from the Profile tab mid-fetch. Important: set
  // true at the *start* of the effect so React.StrictMode's double-invoke
  // (mount → cleanup → mount) restores the gate on the second mount; refs
  // persist across StrictMode's fake unmount, so setting only on teardown
  // leaves the ref permanently false.
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  useEffect(() => {
    let isMounted = true;
    let unlistenFound: (() => void) | undefined;
    let unlistenUpdate: (() => void) | undefined;
    let unlisten7TV: (() => void) | undefined;

    const setupListeners = async () => {
      const { listen } = await import('@tauri-apps/api/event');

      const unlistenFoundFn = await listen('chat-identity-badges-found', (event: any) => {
        if (!mountedRef.current) return;
        const result = event.payload;
        if (result.success) {
          // Dedupe by (id, version). Belt-and-braces against any path
          // that emits duplicates — seen in the wild as every badge
          // appearing twice in the grid until a hard refresh.
          const seen = new Set<string>();
          const deduped = (result.badges as ChatIdentityBadge[]).filter((b) => {
            const key = `${b.id}-${b.version}`;
            if (seen.has(key)) return false;
            seen.add(key);
            return true;
          });
          setChatIdentityBadges(deduped);
        }
        setIsFetchingIdentity(false);
      });
      if (isMounted) unlistenFound = unlistenFoundFn; else unlistenFoundFn();

      const unlistenUpdateFn = await listen('chat-identity-update-result', (event: any) => {
        if (!mountedRef.current) return;
        const result = event.payload;
        if (result.success) {
          setChatIdentityBadges((prev) => prev.map((b) => ({
            ...b,
            is_selected: b.id === result.badge_id,
          })));
        }
        setUpdatingBadgeId(null);
      });
      if (isMounted) unlistenUpdate = unlistenUpdateFn; else unlistenUpdateFn();

      const unlisten7TVFn = await listen('seventv-connected', () => {
        if (!mountedRef.current) return;
        setSeventvAuthConnected(true);
        setIsConnecting7TV(false);
      });
      if (isMounted) unlisten7TV = unlisten7TVFn; else unlisten7TVFn();
    };

    setupListeners();

    return () => {
      isMounted = false;
      if (unlistenFound) unlistenFound();
      if (unlistenUpdate) unlistenUpdate();
      if (unlisten7TV) unlisten7TV();
    };
  }, []);

  useEffect(() => {
    if (!isAuthenticated || !currentUser) return;

    const loadProfile = async () => {
      // INSTANT FIRST PAINT: render the cached profile synchronously, before any
      // network wait. The 7TV auth validate below is a round-trip to 7TV, and it
      // used to run FIRST — so even a warm cache took a couple seconds to show an
      // unthemed card before everything popped in. Now the cache paints
      // immediately; we only show the loading state when nothing is cached yet.
      const cachedProfile = getProfileFromMemoryCache(currentUser.user_id);
      if (cachedProfile && mountedRef.current) {
        applyProfileData(cachedProfile);
      } else if (mountedRef.current) {
        setIsLoadingBadges(true);
      }

      try {
        const status = (await invoke('get_seventv_auth_status')) as { is_authenticated: boolean };
        // The status is an instant local (JWT-exp) read; if it claims connected,
        // confirm authoritatively with 7TV so a revoked token can't read as
        // connected here while the per-account editor says otherwise. This runs in
        // the background now — it no longer gates the first paint above.
        let connected = status.is_authenticated;
        if (connected) {
          connected = (await invoke('validate_seventv_token')) as boolean;
        }
        if (mountedRef.current) setSeventvAuthConnected(connected);
      } catch {
        if (mountedRef.current) setSeventvAuthConnected(false);
      }

      const channelId = currentStream?.user_id || currentUser.user_id;
      const channelName = currentStream?.user_login || currentUser.login || currentUser.username;

      try {
        const profile = await getFullProfileWithFallback(
          currentUser.user_id,
          currentUser.login || currentUser.username,
          channelId,
          channelName,
        );
        if (mountedRef.current) applyProfileData(profile);
      } catch (e) {
        Logger.error('[ProfileSettings] Failed to load profile:', e);
      }

      if (mountedRef.current) setIsLoadingBadges(false);
    };

    loadProfile();
  }, [isAuthenticated, currentUser]);

  // Load this member's saved loadout (which third-party badges they've chosen
  // to display). Drives the checkmarks in the "Other Badges" picker, the card
  // preview, and what other StreamNook clients render in chat.
  useEffect(() => {
    if (!currentUser?.user_id) return;
    getIdentityWithCache(currentUser.user_id)
      .then((lo) => {
        if (mountedRef.current) {
          setLoadout({ customized: lo.customized, badges: lo.badges });
          setLoadoutLoaded(true);
        }
      })
      .catch(() => {});
  }, [currentUser?.user_id]);

  // Keep the chosen Twitch global badge mirrored into the cross-client loadout
  // (as `twitch:<imageId>`) so other members resolve it in the profile overlay.
  // Self-healing: runs once the loadout has loaded AND a badge is selected, and
  // rewrites whenever the selection changes. Guarded on loadoutLoaded so it never
  // clobbers the (not-yet-loaded) third-party badges. Chat is unaffected (it
  // draws Twitch badges from live IRC tags; the resolve endpoint drops this key).
  useEffect(() => {
    if (!loadoutLoaded || !currentUser?.user_id) return;
    const selected = chatIdentityBadges.find((b) => b.is_selected);
    if (!selected) return;
    const imageId = extractTwitchBadgeImageId(selected.image_url);
    if (!imageId) return;
    const key = `twitch:${imageId}`;
    if (loadout.badges.includes(key)) return; // already in sync
    const nextBadges = [key, ...loadout.badges.filter((k) => !k.startsWith('twitch:'))];
    setLoadout({ customized: true, badges: nextBadges });
    void setIdentity(currentUser.user_id, nextBadges, null, true);
    updateProfilePreview({ bumpBadges: true }); // re-resolve the preview's badge row
  }, [loadoutLoaded, chatIdentityBadges, loadout.badges, currentUser?.user_id]);

  // Load the "use my 7TV paint as my profile theme" preference + premium status
  // (the paint theme is a premium / supporter feature).
  useEffect(() => {
    if (!currentUser?.user_id) return;
    getProfilePrefs(currentUser.user_id)
      .then((p) => {
        if (mountedRef.current) {
          setProfileThemeState(p.profileTheme);
          setHiddenSecs(p.hiddenSections);
        }
      })
      .catch(() => {});
    isSubscriber(currentUser.user_id)
      .then((s) => { if (mountedRef.current) setSubscribed(s); })
      .catch(() => {});
    // Earned accolades gate which achievement atmospheres appear. This effect
    // otherwise runs once per user, so a badge earned mid-session (an event
    // accolade granted while watching) would not unlock its atmosphere until a
    // relaunch. Refetch on window focus so returning to the app surfaces it.
    const uid = currentUser.user_id;
    const loadAccolades = () => {
      getAccolades(uid)
        .then((ids) => { if (mountedRef.current) setEarnedAccolades(new Set(ids)); })
        .catch(() => {});
    };
    loadAccolades();
    window.addEventListener('focus', loadAccolades);
    return () => window.removeEventListener('focus', loadAccolades);
  }, [currentUser?.user_id]);

  const selectProfileTheme = (
    id: string,
    tier: 'free' | 'supporter' | 'subscriber',
    bypassTier = false,
  ) => {
    if (!currentUser?.user_id) return;
    if (!bypassTier && !tierMet(tier)) return; // locked until the required tier is met
    setProfileThemeState(id);
    void setProfileTheme(currentUser.user_id, id);
    // Keep the cached profile snapshot's theme in sync so the profile CARD (ours
    // and anyone else's) reflects the change on the next open, instead of serving
    // the old atmosphere until the lazy stale-rewrite. Without this the stale
    // snapshot could also clobber the fresh prefs read on our own card.
    void patchProfileSnapshotTheme(currentUser.user_id, id);
    // Push the new theme to chat immediately (no Supabase read race) so our own
    // messages update in real time; switching away from an Atmosphere clears it.
    refreshAtmosphere(currentUser.user_id, getAtmosphere(id) ? id : null);
    updateProfilePreview({ profileTheme: id }); // live-update an open preview
  };

  // A locked cosmetic doubles as a shortcut: clicking it opens the support page
  // in the browser with the right tier (and the member's Twitch handle) already
  // filled in, so they can grab it in a couple taps instead of hunting the site.
  const openSupportFor = (tier: 'supporter' | 'subscriber') => {
    void (async () => {
      try {
        const { open } = await import('@tauri-apps/plugin-shell');
        const userLogin = currentUser?.login || currentUser?.username;
        const handle = userLogin ? `&handle=${encodeURIComponent(userLogin)}` : '';
        await open(`https://streamnook.app/support?tier=${tier}${handle}`);
      } catch {
        /* opening the browser is best-effort */
      }
    })();
  };

  const toggleSectionVisibility = (key: string) => {
    if (!currentUser?.user_id) return;
    const next = hiddenSecs.includes(key) ? hiddenSecs.filter((k) => k !== key) : [...hiddenSecs, key];
    setHiddenSecs(next);
    void setHiddenSections(currentUser.user_id, next);
    updateProfilePreview({ hiddenSections: next }); // live-update an open preview
  };

  // Open the REAL public-profile overlay (exactly what other members see) for
  // ourselves, seeded with the values we're editing. It then live-updates as we
  // toggle sections / change the theme / edit badges. No backdrop, so the editor
  // stays interactive underneath.
  const showLivePreview = () => {
    if (!currentUser?.user_id) return;
    openProfilePreview(currentUser.user_id, { hiddenSections: hiddenSecs, profileTheme });
  };

  // Resolve YOUR BTTV Pro badge so it can be offered as a toggle in the picker
  // below. Pro is WebSocket-only and not in the server-resolved third-party list,
  // so we fetch it on its own; null means no Pro (toggle hidden).
  useEffect(() => {
    if (!currentUser?.user_id) return;
    let cancelled = false;
    void resolveBttvProUrl(currentUser.user_id).then((url) => {
      if (!cancelled) setSelfBttvProBadge(url ? buildBttvProBadge(url) : null);
    });
    return () => { cancelled = true; };
  }, [currentUser?.user_id]);

  const applyProfileData = (profile: CachedProfile) => {
    setTwitchBadges(profile.twitchBadges);
    // A fresh EMPTY third-party set is a failed resolve (every provider down,
    // or the Rust cache not warm yet), not a member with no badges: the
    // write-through below persists whatever is set here, so applying an empty
    // list would wipe the snapshot the next cold open paints from. Keep the
    // last good set until a non-empty answer arrives.
    setThirdPartyBadges((prev) => {
      if (profile.thirdPartyBadges.length === 0 && prev.length > 0) {
        Logger.warn('[ProfileSettings] Third-party badge resolve returned nothing; keeping the previous set');
        return prev;
      }
      return profile.thirdPartyBadges;
    });
    setSeventvUserId(profile.seventvCosmetics.seventvUserId || null);
    setHas7TVAccountChecked(true);

    // A 7TV hard failure (network / 5xx) surfaces here as an empty cosmetics set.
    // Don't let that strip a paint/badge we're already showing (from the seed
    // cache or an earlier success) down to nothing — keep the last good set until
    // 7TV answers cleanly again. A fresh non-empty set IS authoritative: apply it,
    // and clear the paint when nothing in it is selected (an unequip elsewhere).
    const paints = profile.seventvCosmetics.paints as SevenTVPaint[];
    const badges = profile.seventvCosmetics.badges;
    const hasFreshCosmetics = paints.length > 0 || badges.length > 0;
    if (hasFreshCosmetics) {
      setSeventvBadges(badges);
      setAllSeventvPaints(paints);
      const selectedPaint = paints.find((p: any) => p.selected);
      setSeventvPaint((selectedPaint as SevenTVPaint) ?? null);
    }
  };

  // Write-through: keep the local snapshot in lockstep with whatever the card is
  // rendering, so the next cold open paints this exact loadout instantly. Fires on
  // every cosmetic edit (paint / badge / theme / loadout) AND on the background
  // revalidation results, since both flow through these states. Gated so the empty
  // defaults during the pre-load window can't overwrite a good cached snapshot.
  useEffect(() => {
    const uid = currentUser?.user_id;
    if (!uid) return;
    const hasContent =
      seededProfile != null ||
      loadoutLoaded ||
      twitchBadges.length > 0 ||
      seventvPaint != null ||
      seventvBadges.length > 0 ||
      thirdPartyBadges.length > 0 ||
      chatIdentityBadges.length > 0 ||
      allSeventvPaints.length > 0;
    if (!hasContent) return;
    writeOwnProfileCache(uid, {
      seventvPaint,
      seventvBadges,
      allSeventvPaints,
      seventvUserId,
      twitchBadges,
      thirdPartyBadges,
      chatIdentityBadges,
      selfBttvProBadge,
      loadout,
      profileTheme,
      hiddenSections: hiddenSecs,
    });
  }, [
    currentUser?.user_id,
    seededProfile,
    loadoutLoaded,
    seventvPaint,
    seventvBadges,
    allSeventvPaints,
    seventvUserId,
    twitchBadges,
    thirdPartyBadges,
    chatIdentityBadges,
    selfBttvProBadge,
    loadout,
    profileTheme,
    hiddenSecs,
  ]);

  // Re-evaluate whether the card has anything worth animating. Runs
  // after each render that updates the card's content (paint, badges,
  // tier change). rAF defers until React has committed the new DOM so
  // detectAnimatedPaint sees current pixels, not stale ones. Without
  // this the Animated button could stay disabled after a user just
  // applied an animated paint.
  useEffect(() => {
    if (!profileCardRef.current) return;
    const raf = requestAnimationFrame(() => {
      if (profileCardRef.current && mountedRef.current) {
        setHasAnimatedElements(detectAnimatedPaint(profileCardRef.current));
      }
    });
    return () => cancelAnimationFrame(raf);
  }, [
    currentUser?.user_id,
    seventvPaint,
    chatIdentityBadges,
    seventvBadges,
    streamNookUserNumber,
    activeCosmeticSlug,
  ]);

  useEffect(() => {
    seventvBadges.forEach((badge: any) => {
      if (badge?.id && !badge.localUrl) {
        const badgeUrl = `https://cdn.7tv.app/badge/${badge.id}/4x.webp`;
        queueCosmeticForCaching(badge.id, badgeUrl);
      }
    });

    if ((seventvPaint as any)?.data?.layers) {
      ((seventvPaint as any).data.layers as any[]).forEach((layer: any) => {
        if (layer.ty?.__typename === 'PaintLayerTypeImage' && layer.ty.images) {
          const img = layer.ty.images.find((i: any) => i.scale === 1) || layer.ty.images[0];
          if (img && !img.localUrl) {
            queueCosmeticForCaching(layer.id, img.url);
          }
        }
      });
    }
  }, [seventvBadges, seventvPaint]);

  const fetchChatIdentity = async (showSpinner = true) => {
    if (!currentUser?.login) return;
    if (showSpinner) setIsFetchingIdentity(true);
    try {
      const result = (await invoke('fetch_chat_identity_badges', {
        channelName: currentUser.login,
      })) as { success: boolean; message: string; badges: ChatIdentityBadge[] };
      // The GQL fast path returns the badges inline AND emits
      // 'chat-identity-badges-found'. Use the RETURN VALUE here so a freshly
      // mounted panel never depends on racing the async listener registration —
      // that race (event fires before `await listen(...)` finishes) is what left
      // the spinner stuck on "Loading badges..." forever, intermittently. The
      // browser-scrape fallback returns an empty list and delivers later via the
      // event, by which point the listener has long been registered.
      if (mountedRef.current && result?.success && result.badges?.length) {
        const seen = new Set<string>();
        const deduped = result.badges.filter((b) => {
          const key = `${b.id}-${b.version}`;
          if (seen.has(key)) return false;
          seen.add(key);
          return true;
        });
        setChatIdentityBadges(deduped);
        setIsFetchingIdentity(false);
      }
    } catch {
      if (mountedRef.current) setIsFetchingIdentity(false);
    }
  };

  useEffect(() => {
    if (!currentUser?.user_id) return;

    if (chatIdentityCache &&
        chatIdentityCache.userId === currentUser.user_id &&
        chatIdentityCache.badges.length > 0) {
      Logger.debug('[ProfileSettings] Loading chat identity badges from shared cache:', chatIdentityCache.badges.length);
      setChatIdentityBadges(chatIdentityCache.badges);

      const cacheAge = Date.now() - chatIdentityCache.lastFetched;

      if (cacheAge < CHAT_IDENTITY_BACKGROUND_REFRESH_TTL) return;
      if (cacheAge < CHAT_IDENTITY_CACHE_TTL) {
        fetchChatIdentity(false);
        return;
      }
      fetchChatIdentity(true);
      return;
    }

    fetchChatIdentity(true);
  }, [currentUser?.user_id]);

  const updateChatIdentity = async (badge: ChatIdentityBadge) => {
    if (!currentUser?.login || updatingBadgeId) return;
    setUpdatingBadgeId(badge.id);
    try {
      await invoke('update_chat_identity', {
        channelName: currentUser.login,
        badgeId: badge.id,
        badgeVersion: badge.version,
      });
    } catch {
      if (mountedRef.current) setUpdatingBadgeId(null);
    }
  };

  const handleSelectSeventvPaint = async (paint: SevenTVPaint | null) => {
    if (!seventvUserId || updatingSeventvPaintId) return;

    const paintId = paint?.id || null;
    setUpdatingSeventvPaintId(paintId || 'none');

    try {
      const result = (await invoke('set_seventv_paint', { userId: seventvUserId, paintId })) as { success: boolean };
      if (result.success && mountedRef.current) {
        setSeventvPaint(paint);
        setAllSeventvPaints((prev) => prev.map((p) => ({ ...p, selected: p.id === paintId })));
        // Reflect the change everywhere NOW. 7TV's read API lags the mutation by
        // a few seconds, so a re-fetch here would re-cache the OLD paint (the
        // change wouldn't show until a reload). Instead flip the selection on the
        // cached cosmetics so chat rows + profile card repaint at once.
        // clear7TVCache drops the lower-level 7TV-service entry so a later natural
        // fetch is fresh once 7TV is consistent (prevents stale resurrection).
        clear7TVCache();
        if (currentUser?.user_id) {
          const applied = applyLocalCosmeticSelection(currentUser.user_id, { paintId });
          if (!applied) {
            // Nothing cached to flip — fall back to a fresh fetch.
            invalidateUserCosmetics(currentUser.user_id);
            void getCosmeticsWithFallback(currentUser.user_id).catch(() => {});
          }
        }
      }
    } catch (e: unknown) {
      Logger.error('[ProfileSettings] Failed to update paint:', e);
      const errMsg = e instanceof Error ? e.message : typeof e === 'string' ? e : '';
      if (errMsg.includes('SESSION_EXPIRED') && mountedRef.current) {
        setSeventvAuthConnected(false);
        setSeventvUserId(null);
      }
    } finally {
      if (mountedRef.current) setUpdatingSeventvPaintId(null);
    }
  };

  const handleSelectSeventvBadge = async (badge: SevenTVBadge | null) => {
    if (!seventvUserId || updatingSeventvBadgeId) return;

    const badgeId = badge?.id || null;
    setUpdatingSeventvBadgeId(badgeId || 'none');

    try {
      const result = (await invoke('set_seventv_badge', { userId: seventvUserId, badgeId })) as { success: boolean };
      if (result.success && mountedRef.current) {
        setSeventvBadges((prev) => prev.map((b) => ({ ...b, selected: b.id === badgeId })));
        // Optimistic flip (same rationale as paint): 7TV's read lags the write,
        // so re-fetching would re-cache the old badge. clear7TVCache keeps the
        // lower-level entry fresh for a later natural fetch.
        clear7TVCache();
        if (currentUser?.user_id) {
          const applied = applyLocalCosmeticSelection(currentUser.user_id, { badgeId });
          if (!applied) {
            invalidateUserCosmetics(currentUser.user_id);
            void getCosmeticsWithFallback(currentUser.user_id).catch(() => {});
          }
        }
        updateProfilePreview({ bumpBadges: true }); // re-resolve the preview's badge row
      }
    } catch (e: unknown) {
      Logger.error('[ProfileSettings] Failed to update badge:', e);
      const errMsg = e instanceof Error ? e.message : typeof e === 'string' ? e : '';
      if (errMsg.includes('SESSION_EXPIRED') && mountedRef.current) {
        setSeventvAuthConnected(false);
        setSeventvUserId(null);
      }
    } finally {
      if (mountedRef.current) setUpdatingSeventvBadgeId(null);
    }
  };

  const handleOpen7TVCosmetics = async () => {
    if (!seventvUserId) return;
    const { open } = await import('@tauri-apps/plugin-shell');
    await open(`https://7tv.app/users/${seventvUserId}/cosmetics`);
  };

  const handleOpenTwitchProfile = async () => {
    if (!currentUser?.login) return;
    const { open } = await import('@tauri-apps/plugin-shell');
    await open(`https://twitch.tv/${currentUser.login}`);
  };

  const handleShareProfile = async (mode: CaptureMode) => {
    if (!profileCardRef.current || isSharing) return;
    setIsSharing(true);
    setCaptureStage('recording');
    setCaptureProgress(0);

    // Fresh pull before capture: clear the cosmetics memory cache and
    // re-fetch so the card reflects current backend state, not whatever
    // was cached from an earlier session. Cheap if data hasn't actually
    // changed (network round-trips, but no DOM work if results match).
    if (currentUser?.user_id) {
      try {
        clearCosmeticsMemoryCache();
        const channelId = currentStream?.user_id || currentUser.user_id;
        const channelName =
          currentStream?.user_login || currentUser.login || currentUser.username;
        const fresh = await getFullProfileWithFallback(
          currentUser.user_id,
          currentUser.login || currentUser.username,
          channelId,
          channelName,
        );
        if (mountedRef.current) applyProfileData(fresh);
      } catch (e) {
        // Non-fatal — proceed with whatever's already rendered.
        Logger.warn('[ProfileSettings] Profile refresh before share failed:', e);
      }
    }

    // For static (PNG) captures, freeze CSS animations on the card so
    // the screenshot lands on a stable frame instead of an in-between
    // moment that can look like the paint is torn or glitching. Animated
    // captures want the animation running, so skip the pause there.
    let restoreAnimations: (() => void) | null = null;
    if (mode === 'static' && profileCardRef.current) {
      restoreAnimations = pauseCardAnimations(profileCardRef.current);
    }

    // Double-RAF so React commits the hidden-button state to the DOM AND
    // the compositor repaints before we ask Rust to grab the screen
    // pixels. Also gives the paused animation-play-state above time to
    // commit. Without this, the Share buttons show up in the capture
    // because the visibility change hasn't reached the screen buffer yet.
    await new Promise<void>((r) =>
      requestAnimationFrame(() => requestAnimationFrame(() => r())),
    );

    // Subscribe to the diagnostic stats event so we can include actual
    // achieved frame count / fps in the success toast.
    type CaptureStats = {
      frame_count: number;
      capture_ms: number;
      encode_ms: number;
      duration_ms: number;
    };
    let captureStats: CaptureStats | null = null;
    const { listen } = await import('@tauri-apps/api/event');
    const unlistenStats = await listen<CaptureStats>(
      'profile-capture-stats',
      (e) => {
        captureStats = e.payload;
      },
    );

    // Progress UI driver. We know the recording phase runs for
    // `expectedDuration` (the source animation cycle, capped at the WebP
    // max). Past that, capture is done and the encode phase starts — we
    // flip to the indeterminate 'finalizing' spinner so the user knows
    // something is still happening but doesn't see a stalled progress
    // bar.
    const expectedDuration = estimateCaptureDurationMs(profileCardRef.current);
    const progressStart = Date.now();
    const progressInterval = window.setInterval(() => {
      if (!mountedRef.current) return;
      const elapsed = Date.now() - progressStart;
      if (elapsed < expectedDuration) {
        const pct = Math.min(99, Math.round((elapsed / expectedDuration) * 100));
        setCaptureProgress(pct);
      } else {
        setCaptureStage((s) => (s === 'recording' ? 'finalizing' : s));
      }
    }, 50);

    try {
      const result = await captureProfileCard(profileCardRef.current, { mode });
      const label = result.mime === 'image/gif' ? 'GIF' : 'image';
      const statsSuffix = (() => {
        if (!captureStats || result.mime !== 'image/webp') return '';
        const { frame_count, capture_ms } = captureStats as CaptureStats;
        const fps =
          capture_ms > 0 ? Math.round((frame_count * 1000) / capture_ms) : 0;
        return ` (${frame_count} frames, ${fps}fps)`;
      })();
      const copied = await copyImageToClipboard(result.blob, result.mime);
      if (copied) {
        addToast(`Profile ${label} copied to clipboard${statsSuffix}`, 'success');
      } else {
        // Clipboard rejected the MIME (common for image/gif on some platforms).
        // Fall back to triggering a download so the user can drag the file
        // into Discord instead.
        downloadBlob(result.blob, result.filename);
        addToast(
          `Profile ${label} saved to downloads${statsSuffix}`,
          'success',
        );
      }
    } catch (e) {
      Logger.error('[ProfileSettings] Share profile failed:', e);
      addToast('Failed to capture profile', 'error');
    } finally {
      restoreAnimations?.();
      window.clearInterval(progressInterval);
      unlistenStats();
      if (mountedRef.current) {
        setIsSharing(false);
        setCaptureStage(null);
        setCaptureProgress(0);
      }
    }
  };

  const selectedGlobalBadge = chatIdentityBadges.find((b) => b.is_selected);
  const selected7TVBadge = seventvBadges.find((b: any) => b.selected);

  // ── Third-party loadout (the cross-client "what shows in chat" selection) ──
  // Opt-in: a third-party badge appears in the card + chat ONLY when explicitly
  // selected. This matches the baseline (third-party badges aren't shown in chat
  // by default) and keeps partial editors — like the website, which only resolves
  // some providers — safe: each editor adds/removes only its own keys and
  // preserves every other key already in the loadout.
  const tpKey = (b: { provider: string; id: string }) => `${b.provider}:${b.id}`;
  const isThirdPartyShown = (b: { provider: string; id: string }) =>
    loadout.badges.includes(tpKey(b));
  // Render the shown badges in the member's CHOSEN order (their stored loadout
  // key order) so this preview matches chat + the profile card, which both honor
  // that order. Filtering thirdPartyBadges directly kept provider/fetch order and
  // is what made the preview disagree with chat.
  // The full toggleable set = the server-resolved third-party badges PLUS your
  // own BTTV Pro (resolved client-side; absent for non-Pro users). Drives both
  // the preview ordering here and the picker grid below.
  const allThirdParty: any[] = selfBttvProBadge
    ? [...thirdPartyBadges, selfBttvProBadge]
    : thirdPartyBadges;
  const shownThirdPartyByKey = new Map<string, any>(
    allThirdParty.map((b) => [tpKey(b as any), b]),
  );
  const shownThirdParty = loadout.badges
    .map((k) => shownThirdPartyByKey.get(k))
    .filter((b): b is NonNullable<typeof b> => b != null);
  const toggleThirdParty = (b: { provider: string; id: string }) => {
    if (!currentUser?.user_id) return;
    const key = tpKey(b);
    const nextBadges = loadout.badges.includes(key)
      ? loadout.badges.filter((k) => k !== key)
      : [...loadout.badges, key];
    const next = { customized: true, badges: nextBadges };
    setLoadout(next);
    void setIdentity(currentUser.user_id, next.badges, null, true);
    updateProfilePreview({ bumpBadges: true }); // re-resolve the preview's badge row
  };
  const tier = streamNookUserNumber !== null ? getTier(streamNookUserNumber) : null;
  // The real tier accent (its color), so the "Tier aura" swatch + preview match
  // what the profile actually shows instead of a generic gray.
  const tierAccent = streamNookUserNumber !== null ? getTierAccent(streamNookUserNumber) : null;

  // Tiered unlocks (paint at supporter, atmospheres at subscriber), resolved in
  // one place. Owning the subscriber badge is the permanent proof of tier;
  // active status is only a fast path.
  const { canPaint, canAtmosphere, tierMet } = resolveEntitlement({
    ownedSlugs: ownedCosmeticSlugs,
    activeSubscription: subscribed,
  });

  // Base profile-BACKGROUND options: tier (free) + the member's 7TV paint
  // (supporter). StreamNook Atmospheres are their OWN cosmetic section (subscriber,
  // also decorates chat), not lumped in here.
  const baseThemeOptions: Array<{
    id: string;
    name: string;
    tier: 'free' | 'supporter' | 'subscriber';
    swatch: CSSProperties;
  }> = [
    {
      id: 'tier',
      name: 'Tier aura',
      tier: 'free',
      swatch: tierAccent
        ? {
            backgroundColor: '#0d0e14',
            backgroundImage: `radial-gradient(ellipse 130% 95% at 50% 12%, rgba(${tierAccent.rgb}, 0.6), transparent 62%)`,
          }
        : { background: 'linear-gradient(135deg, rgba(226,232,240,0.20), rgba(148,163,184,0.10))' },
    },
    ...(seventvPaint
      ? [
          {
            id: 'paint',
            name: '7TV Paint',
            tier: 'supporter' as const,
            swatch: ((): CSSProperties => {
              const p = computePaintStyle(seventvPaint as any, '#9146FF');
              return {
                backgroundImage: p.backgroundImage,
                backgroundColor:
                  typeof p.backgroundColor === 'string' && !p.backgroundColor.startsWith('var')
                    ? p.backgroundColor
                    : undefined,
                backgroundSize: 'cover',
              };
            })(),
          },
        ]
      : []),
  ];

  // Whether an Atmosphere is available to this member. Accolade-gated ones
  // (e.g. Midnight via the Insomniac accolade) unlock for ANY member who earned
  // the accolade, regardless of subscription. Subscriber atmospheres are owned
  // per-item: you keep every one you unlocked, and an active subscriber can
  // apply (and thereby keep) new ones. Lapsing freezes you to what you own.
  const atmUnlocked = (a: Atmosphere): boolean =>
    // Dev-only candidate slots are always pickable on a dev build; that is
    // what they are for. No-op in production (the set is empty there).
    isDevAtmosphere(a.id)
      ? true
      : a.unlock?.kind === 'accolade'
        ? earnedAccolades.has(a.unlock.accoladeId)
        : ownedCosmeticSlugs.has(a.id) || subscribed;

  // How an atmosphere is earned, shown as the picker tooltip. Accolade-gated
  // ones only appear once earned, so this reads as "here's how you got it";
  // named per accolade so it points at the right medallion on the wall. These
  // are accolades (achievements), never "badges" (the chat cosmetics).
  const atmosphereUnlockNote = (a: Atmosphere): string | null => {
    if (a.unlock?.kind === 'accolade') {
      const accolades: Record<string, string> = {
        semiquincentennial_2026: 'Semiquincentennial',
        // Stored id is the original codename; the player-facing name is Restless.
        insomniac: 'Restless',
        cs2_major_cologne_2026: 'CS2 Major Cologne 2026',
      };
      const name = accolades[a.unlock.accoladeId];
      return name ? `Unlocked by the ${name} accolade` : 'Unlocked by an accolade';
    }
    if (a.unlock?.kind === 'subscriber') return 'Subscriber atmosphere';
    return null;
  };

  // Live preview: the hovered cosmetic (or the active one when nothing hovered).
  const activePreviewId = previewThemeId ?? profileTheme;
  const previewAtm = getAtmosphere(activePreviewId);
  // Frosted readability block behind the text for busy washes (e.g. Midnight),
  // mirroring ChatMessage so the text stays legible over the image in the preview.
  const previewFrost = !!previewAtm?.chatFrost;
  const previewLocked = previewAtm ? !atmUnlocked(previewAtm) : activePreviewId === 'paint' ? !canPaint : false;
  // Name the RIGHT tier on the lock pill: an Atmosphere needs Subscriber, but a
  // 7TV paint only needs Supporter. Telling a paint previewer to "subscribe"
  // overcharges them (Supporter is the cheaper one-time tier that unlocks it).
  const previewLockTier: 'supporter' | 'subscriber' = previewAtm ? 'subscriber' : 'supporter';
  // A fresh random sample line (+ maybe a 7TV emote) each time the previewed
  // cosmetic changes, so the mock chat reads like real, varied chat.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const previewChat = useMemo(() => rollPreviewChat(previewEmotes), [activePreviewId, previewEmotes]);

  if (!isAuthenticated || !currentUser) {
    return (
      <div className="flex h-full min-h-[420px] items-center justify-center">
        <div className="glass-panel rounded-xl p-8 max-w-sm w-full text-center">
          <p className="text-sm text-textSecondary mb-6 leading-relaxed">
            Sign in to manage your Twitch badges, 7TV cosmetics, and StreamNook identity.
          </p>
          <button
            onClick={loginToTwitch}
            className="w-full px-5 py-2.5 bg-[#9146FF] hover:bg-[#772CE8] text-white text-sm font-medium rounded-lg transition-colors flex items-center justify-center gap-2"
          >
            <svg fill="currentColor" viewBox="0 0 512 512" className="w-4 h-4">
              <path d="M80,32,48,112V416h96v64h64l64-64h80L464,304V32ZM416,288l-64,64H256l-64,64V352H112V80H416Z" />
              <rect x="320" y="143" width="48" height="129" />
              <rect x="208" y="143" width="48" height="129" />
            </svg>
            Sign in with Twitch
          </button>
        </div>
      </div>
    );
  }

  // sn-light-off: this tab previews the user's OWN cosmetics (atmospheres,
  // paints, badges), so a theme's light treatment must not paint under them.
  return (
    <div className="sn-light-off space-y-6">
      {/* Share controls rendered into the dialog hero's actions slot
          (#settings-hero-actions). Same vertical row as the "Profile"
          title/description, on the right side. Zero impact on the layout
          below — the card stays at the top of the content area as if
          no share controls existed. Tooltips side="bottom" so they
          render below the hero (into the empty space above the card,
          which is OUTSIDE DXGI's capture rect on the card). */}
      {heroActionsTarget &&
        createPortal(
          <>
            {/* Small "Share" label before the icons so the affordance is
                obvious to users who don't recognize the icons alone.
                Same uppercase / tracking treatment as the StreamNook tier
                labels and other dialog accents — reads as a section tag
                rather than body copy. */}
            <span className="mr-1.5 text-[10px] font-medium uppercase tracking-[0.12em] text-textMuted">
              Share
            </span>
            <Tooltip
              content="Copy as still image"
              side="bottom"
              disabled={isSharing}
            >
              <button
                type="button"
                onClick={() => handleShareProfile('static')}
                disabled={isSharing}
                aria-label="Copy as still image"
                className="p-1.5 text-textMuted hover:text-textPrimary hover:bg-white/[0.05] rounded-md transition-colors disabled:cursor-wait"
              >
                <ImageIcon size={16} />
              </button>
            </Tooltip>
            <Tooltip
              content={
                hasAnimatedElements
                  ? 'Copy as animated WebP'
                  : 'Nothing animated on this card'
              }
              side="bottom"
              disabled={isSharing}
            >
              <button
                type="button"
                onClick={() => handleShareProfile('animated')}
                disabled={isSharing || !hasAnimatedElements}
                aria-label="Copy as animated WebP"
                className="p-1.5 text-textMuted hover:text-textPrimary hover:bg-white/[0.05] rounded-md transition-colors disabled:cursor-not-allowed disabled:text-textMuted/30 disabled:hover:bg-transparent disabled:hover:text-textMuted/30"
              >
                <Film size={16} />
              </button>
            </Tooltip>
          </>,
          heroActionsTarget,
        )}

      {/* Inner Profile tabs: Overview (stats showcase) vs Customize (identity
          editor). The profile card below stays visible on both. */}
      <div className="flex w-fit items-center gap-1 rounded-lg border border-white/[0.06] bg-white/[0.03] p-1">
        {(['overview', 'customize'] as const).map((t) => (
          <button
            key={t}
            type="button"
            onClick={() => setProfileTab(t)}
            className={`rounded-md px-3 py-1.5 text-[13px] font-medium capitalize transition-colors ${
              profileTab === t
                ? 'bg-white/[0.08] text-textPrimary'
                : 'text-textSecondary hover:text-textPrimary'
            }`}
          >
            {t}
          </button>
        ))}
      </div>

      <div ref={profileCardRef} className="relative overflow-hidden flex items-center gap-8 p-6 glass-panel rounded-xl">
        {/* Tier aura backdrop — Ethereal gets violet, Mythic gets amber.
            Other tiers don't define one; the card stays neutral glass. */}
        {tier?.auraClassName && <div className={tier.auraClassName} />}
        <div className="relative w-32 h-32 rounded-full bg-white/[0.04] flex-shrink-0 flex items-center justify-center overflow-hidden ring-1 ring-inset ring-white/10 shadow-[0_6px_20px_rgba(0,0,0,0.45)]">
          {currentUser.profile_image_url ? (
            <img src={currentUser.profile_image_url} alt="Profile" className="w-full h-full object-cover" />
          ) : (
            <User size={56} className="text-textSecondary" />
          )}
        </div>

        <div className="relative flex-1 min-w-0">
          <div className="flex items-center gap-2 mb-2 flex-wrap">
            {/* Canonical badge order (see utils/badgeOrder): StreamNook leads, then
                Twitch global, then 7TV, then third-party. The card has no channel
                context, so the channel-contextual tier (sub/poll) never shows here. */}
            {streamNookUserNumber !== null && currentUser?.user_id && (
              <StreamNookBadge userId={currentUser.user_id} userNumber={streamNookUserNumber} side="bottom" />
            )}
            {selectedGlobalBadge && (
              <Tooltip content={`Twitch: ${selectedGlobalBadge.title}`} side="top">
                <img src={selectedGlobalBadge.image_url} alt={selectedGlobalBadge.title} className="w-6 h-6" />
              </Tooltip>
            )}
            {selected7TVBadge && (() => {
              const urls = getBadgeImageUrls(selected7TVBadge as any);
              return urls.url4x ? (
                <Tooltip content={`7TV: ${selected7TVBadge.tooltip || selected7TVBadge.name}`} side="top">
                  <FallbackImage
                    src={urls.url4x}
                    fallbackUrls={getBadgeFallbackUrls(selected7TVBadge.id).slice(1)}
                    alt={selected7TVBadge.tooltip || selected7TVBadge.name}
                    className="w-6 h-6"
                  />
                </Tooltip>
              ) : null;
            })()}
            {/* Selected third-party badges — the card is the screenshot preview,
                so it shows exactly the loadout other StreamNook users will see. */}
            {shownThirdParty.map((badge: any) => (
              <Tooltip key={`card-tp-${badge.provider}-${badge.id}`} content={`${badge.title} (${badge.provider.toUpperCase()})`} side="top">
                <img src={badge.image4x || badge.imageUrl} alt={badge.title} className="w-6 h-6" />
              </Tooltip>
            ))}
            <h3
              className="text-3xl font-bold"
              style={seventvPaint ? computePaintStyle(seventvPaint as any, '#9146FF') : { color: 'var(--text-primary)' }}
            >
              {currentUser.display_name || currentUser.login}
            </h3>
            {currentUser.broadcaster_type === 'partner' && (
              <svg className="w-5 h-5 flex-shrink-0" viewBox="0 0 16 16" fill="#9146FF">
                <path fillRule="evenodd" d="M12.5 3.5 8 2 3.5 3.5 2 8l1.5 4.5L8 14l4.5-1.5L14 8l-1.5-4.5ZM7 11l4.5-4.5L10 5 7 8 5.5 6.5 4 8l3 3Z" clipRule="evenodd" />
              </svg>
            )}
            {seventvPaint && (
              <Tooltip content={`Paint: ${seventvPaint.name}`} side="top">
                <button
                  type="button"
                  onClick={() => openBadgesWithPaintInMain(seventvPaint.id)}
                  className="px-2 py-0.5 rounded-md text-[11px] font-bold inline-block relative overflow-hidden cursor-pointer hover:ring-1 hover:ring-accent/50 transition-all border border-transparent shadow-[inset_1px_1px_0_0_rgba(255,255,255,0.10),inset_-1px_-1px_0_0_rgba(0,0,0,0.18)]"
                  style={{
                    ...computePaintStyle(seventvPaint as any, '#9146FF'),
                    WebkitBackgroundClip: 'padding-box',
                    backgroundClip: 'padding-box',
                  }}
                >
                  <span
                    style={{
                      ...computePaintStyle(seventvPaint as any, '#9146FF'),
                      filter: 'invert(1) contrast(1.5)',
                      WebkitBackgroundClip: 'text',
                      backgroundClip: 'text',
                    }}
                  >
                    {seventvPaint.name}
                  </span>
                </button>
              </Tooltip>
            )}
          </div>
          <p className="text-textSecondary text-base">@{currentUser.login}</p>
        </div>

        {tier && streamNookUserNumber !== null && (
          <div className="relative flex-shrink-0 flex flex-col items-center pt-1 pb-2 min-w-[120px]">
            <div className="text-[9px] uppercase tracking-[0.36em] font-medium text-white/35 mb-3">
              StreamNook
            </div>
            <div className="flex items-baseline justify-center gap-1.5 mb-3">
              <span className="text-[11px] text-white/30 font-light leading-none">Nº</span>
              <span className={tier.numberClassName}>{streamNookUserNumber}</span>
              <span className="text-[11px] font-light leading-none invisible" aria-hidden="true">
                Nº
              </span>
            </div>
            <div className={`w-12 h-px ${tier.hairlineClassName} mb-2`} />
            {tier.label && <div className={tier.labelClassName}>{tier.label}</div>}
          </div>
        )}
        </div>

      {/* Intro for the identity editor. Shown only on the Customize sub-tab;
          the Overview sub-tab renders its own stats view instead. */}
      {profileTab === 'customize' && (
        <div className="flex items-center justify-between gap-3">
          <p className="text-xs text-textSecondary">
            Customize your identity. Choose what shows next to your name in chat.
          </p>
          <button
            type="button"
            onClick={showLivePreview}
            className="flex-shrink-0 inline-flex items-center rounded-md bg-emerald-500/15 px-3 py-1.5 text-xs font-semibold text-emerald-300 transition-colors hover:bg-emerald-500/25"
          >
            Show live preview
          </button>
        </div>
      )}

      {/* Capture-progress card. Renders BELOW the profile card so it
          stays outside DXGI's capture rect — anything painted over the
          card area would land in the recorded WebP, so this UI lives in
          the row immediately under it. Two stages: determinate progress
          bar while elapsed < expectedDuration (the active recording
          phase), then indeterminate spinner during the encode tail. */}
      <AnimatePresence>
        {captureStage && (
          <motion.div
            key="capture-progress"
            initial={{ opacity: 0, y: -8, height: 0 }}
            animate={{ opacity: 1, y: 0, height: 'auto' }}
            exit={{ opacity: 0, y: -8, height: 0 }}
            transition={{ duration: 0.18, ease: 'easeOut' }}
            className="overflow-hidden"
          >
            <div className="glass-panel rounded-xl p-4 flex items-center gap-4">
              {captureStage === 'recording' ? (
                <span
                  className="flex-shrink-0 w-2 h-2 rounded-full bg-red-500 animate-pulse"
                  aria-hidden
                />
              ) : (
                <span
                  className="flex-shrink-0 w-3 h-3 border-2 border-white/20 border-t-white/70 rounded-full animate-spin"
                  aria-hidden
                />
              )}
              <div className="flex-1 min-w-0">
                <div className="flex items-baseline justify-between mb-1.5">
                  <span className="text-sm font-medium text-textPrimary">
                    {captureStage === 'recording'
                      ? 'Recording your profile…'
                      : 'Finalizing…'}
                  </span>
                  {captureStage === 'recording' && (
                    <span className="text-xs tabular-nums text-textMuted">
                      {captureProgress}%
                    </span>
                  )}
                </div>
                <div className="h-1 bg-white/[0.06] rounded-full overflow-hidden mb-2">
                  <div
                    className={`h-full transition-[width] duration-100 ease-linear ${
                      captureStage === 'recording'
                        ? 'bg-red-500/80'
                        : 'bg-white/60'
                    }`}
                    style={{
                      width: captureStage === 'recording' ? `${captureProgress}%` : '100%',
                    }}
                  />
                </div>
                <p className="text-[11px] text-textMuted leading-snug">
                  Keep this window visible — switching apps or covering the card
                  will appear in the capture.
                </p>
              </div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {profileTab === 'overview' && (
        <>
          <div className="flex items-center justify-between gap-3 px-1">
            <p className="text-[11px] leading-relaxed text-textSecondary">
              This is your private view. Your hidden sections still show here. To
              see exactly what other members see, open the live preview.
            </p>
            <button
              type="button"
              onClick={showLivePreview}
              className="flex-shrink-0 inline-flex items-center rounded-md bg-emerald-500/15 px-3 py-1.5 text-xs font-semibold text-emerald-300 transition-colors hover:bg-emerald-500/25"
            >
              Show live preview
            </button>
          </div>
          <ProfileOverview
            userId={currentUser.user_id}
            login={currentUser.login || currentUser.username || ''}
            broadcasterType={currentUser.broadcaster_type || ''}
            streamNookUserNumber={streamNookUserNumber}
            seventvPaintCount={allSeventvPaints.length}
            seventvBadgeCount={seventvBadges.length}
            ownedCosmeticsCount={ownedCosmetics.length}
          />

          {/* Accounts belong on the tab Profile actually opens on. This panel
              used to sit at the very bottom of Customize, below badges, paints
              and atmospheres — so "manage my accounts" meant finding it inside a
              cosmetics screen. It is identity, not decoration, and it is now the
              one place Twitch, Kick and YouTube are all managed. */}
          <LinkedAccountsSection />
        </>
      )}

      {profileTab === 'customize' && (
        <>
      <div className="px-1 pt-0.5">
        <h3 className="text-[11px] font-semibold uppercase tracking-[0.14em] text-textMuted">Cosmetics</h3>
        <p className="mt-0.5 text-[11px] leading-relaxed text-textSecondary">
          Your StreamNook identity: badges, your profile background, and animated Atmospheres.
        </p>
      </div>
      {streamNookUserNumber !== null && currentUser?.user_id && (ownedBadges.length > 0 || lockedBadges.length > 0) && (
        <div className="settings-card p-4">
          <div className="flex items-center gap-1.5 mb-4">
            <Tooltip content="Open StreamNook badges" side="top">
              <button
                onClick={openBadgesOnStreamNookInMain}
                className="cursor-pointer hover:scale-110 transition-transform"
                aria-label="Open StreamNook badges"
              >
                <img
                  src={streamNookLogo}
                  alt=""
                  className="w-4 h-4 object-contain"
                  draggable={false}
                />
              </button>
            </Tooltip>
            <h4 className="text-sm font-semibold text-textPrimary uppercase tracking-wide">
              Badges
            </h4>
          </div>
          <div className="flex flex-wrap gap-2">
            {ownedBadges.map((cosmetic) => {
              const asset = resolveCosmeticAsset(cosmetic, { chatSize: true });
              if (!asset) return null;
              const isActive = activeCosmeticSlug === cosmetic.slug;
              // Switch only — never unequip. A StreamNook member always wears a
              // badge so their StreamNook identity stays visible to other members;
              // clicking the already-active badge is a no-op rather than clearing
              // the selection back to none.
              return (
                <Tooltip
                  key={cosmetic.slug}
                  content={
                    <div className="text-center">
                      <div className="font-semibold">{cosmetic.name}</div>
                      {cosmetic.description && (
                        <div className="text-[11px] text-textSecondary mt-0.5">{cosmetic.description}</div>
                      )}
                    </div>
                  }
                  side="top"
                >
                  <div
                    onClick={() => { if (!isActive) void setActiveCosmetic(currentUser.user_id, cosmetic.slug); }}
                    className={`
                      relative p-2 rounded-lg transition-all
                      ${isActive
                        ? 'glass-input border-accent/50 ring-1 ring-accent/30 cursor-default'
                        : 'border border-transparent cursor-pointer hover:bg-glass hover:border-borderLight'}
                    `}
                  >
                    <img
                      src={asset}
                      alt={cosmetic.name}
                      className="w-8 h-8 object-contain"
                      draggable={false}
                    />
                    {isActive && <CheckChip tone="accent" />}
                  </div>
                </Tooltip>
              );
            })}
            {lockedBadges.map((cosmetic) => {
              const asset = resolveCosmeticAsset(cosmetic, { chatSize: true });
              if (!asset) return null;
              const how = cosmetic.payment_type === 'Subscription'
                ? 'Subscriber badge'
                : cosmetic.payment_type === 'Donation'
                  ? 'Supporter badge'
                  : null;
              return (
                <Tooltip
                  key={cosmetic.slug}
                  content={
                    <div className="text-center">
                      <div className="font-semibold">{cosmetic.name}</div>
                      <div className="mt-0.5 inline-flex items-center gap-1 text-[11px] text-amber-300/90">
                        <Lock size={10} />
                        {how ? `Locked. ${how}` : 'Locked'}
                      </div>
                    </div>
                  }
                  side="top"
                >
                  <div
                    aria-disabled
                    className="relative cursor-default rounded-lg border border-dashed border-white/[0.08] p-2"
                  >
                    <img
                      src={asset}
                      alt={cosmetic.name}
                      className="h-8 w-8 object-contain opacity-40 saturate-[0.35]"
                      draggable={false}
                    />
                    <LockChip />
                  </div>
                </Tooltip>
              );
            })}
          </div>
          {cosmeticsCatalog.some((c) => c.is_active && !ownedCosmeticSlugs.has(c.slug)) && (
            <div className="mt-4 flex flex-wrap items-center justify-center gap-2 text-[11px] text-textSecondary">
              <span>Enjoy collecting badges or flexing status?</span>
              <button
                type="button"
                onClick={openBadgesOnStreamNookInMain}
                className="inline-flex items-center gap-1.5 px-3 py-1 rounded-full text-[11px] font-medium text-textPrimary bg-white/[0.06] hover:bg-white/[0.10] transition-colors cursor-pointer"
              >
                <Heart size={10} className="text-rose-400/80" aria-hidden />
                Fund the chaos
              </button>
            </div>
          )}
        </div>
      )}

      {/* Chat preview: how your name + badges read in a chat line for the hovered
          (or active) cosmetic. An Atmosphere themes the chat backdrop; a 7TV paint
          only the name. The full profile is now shown 1:1 by the live preview
          (the "Show live preview" button above), so it's no longer mocked here. */}
      <div className="space-y-2">
        <div className="flex items-center gap-2 px-1">
          <span className="text-[10px] font-semibold uppercase tracking-[0.12em] text-textMuted">Chat preview</span>
          {previewLocked && (
            <button
              type="button"
              onClick={() => openSupportFor(previewLockTier)}
              className="inline-flex items-center gap-1 rounded-full border border-amber-400/30 bg-amber-400/15 px-2 py-0.5 text-[9px] font-semibold uppercase tracking-wide text-amber-200/90 transition-colors hover:bg-amber-400/25"
            >
              Get {previewLockTier === 'supporter' ? 'Supporter' : 'Subscriber'}
              <ExternalLink size={9} className="opacity-70" />
            </button>
          )}
        </div>

        {/* Mock chat message at real chat sizes (text-sm, 20px badges), capped at a
            real chat column width (~402px). Inline flow (below) wraps a long
            message to the left edge like real chat, and the min height keeps it at
            least two lines tall so the atmosphere reads with real vertical presence
            instead of a thin single line. */}
        <div
          className="relative isolate w-full max-w-[402px] overflow-hidden rounded-lg border border-white/[0.06]"
          style={{ paddingLeft: 18, paddingRight: 18, paddingTop: 7, paddingBottom: 7, minHeight: 60 }}
        >
          {previewAtm && <AtmosphereBackground atm={previewAtm} variant="chat" />}
          {/* Inline flow (not flex) so a long message wraps to the LEFT edge on the
              next line like a real chat row, instead of indenting under the name or
              stretching into one super-wide line. Busy washes (chatFrost, e.g.
              Midnight) get the same frosted readability block real chat uses. */}
          <div
            className={`relative text-sm leading-relaxed ${
              previewFrost
                ? 'inline-block max-w-full rounded-md bg-[rgba(5,6,13,0.22)] px-1.5 py-0.5 backdrop-blur-[4px]'
                : ''
            }`}
          >
            <span className="mr-1.5 align-middle text-[11px] text-textMuted">3:45</span>
            {streamNookUserNumber !== null && currentUser?.user_id && (
              <span className="mr-1 inline-flex align-middle">
                <StreamNookBadge userId={currentUser.user_id} userNumber={streamNookUserNumber} side="top" />
              </span>
            )}
            <span
              className="align-middle font-semibold"
              style={seventvPaint ? computePaintStyle(seventvPaint as any, '#9146FF') : undefined}
            >
              {currentUser.display_name || currentUser.login}
            </span>
            <span className="align-middle text-textSecondary">
              {' '}
              {previewChat.text}
              {previewChat.emotes.map((e, i) => (
                <Tooltip key={`${e.id}-${i}`} content={e.name}>
                  <img
                    src={previewEmoteUrl(e.id)}
                    alt={e.name}
                    className="mx-0.5 inline-block h-6 w-auto align-middle object-contain"
                    onError={(ev) => {
                      (ev.currentTarget as HTMLImageElement).style.display = 'none';
                    }}
                  />
                </Tooltip>
              ))}
            </span>
          </div>
        </div>
      </div>

      {/* Profile background: the basic backdrop source (tier or your 7TV paint).
          A paint only changes the BACKGROUND, not chat. */}
      <div className="settings-card p-4">
        <h4 className="text-sm font-semibold text-textPrimary">Profile background</h4>
        <p className="mt-0.5 text-[12px] leading-relaxed text-textSecondary">
          Just your profile background. The tier aura is free; using your equipped 7TV paint is a
          supporter perk. A paint only changes the background, not your chat.
        </p>
        <div className="mt-3 space-y-2">
          {baseThemeOptions.map((opt) => {
            const selected = profileTheme === opt.id;
            const locked = !tierMet(opt.tier);
            return (
              <button
                key={opt.id}
                type="button"
                onClick={() => (locked ? openSupportFor(opt.tier === 'subscriber' ? 'subscriber' : 'supporter') : selectProfileTheme(opt.id, opt.tier))}
                onMouseEnter={() => setPreviewThemeId(opt.id)}
                onMouseLeave={() => setPreviewThemeId(null)}
                className={`flex w-full items-center gap-3 rounded-lg border p-2.5 text-left transition-colors ${
                  selected
                    ? 'border-accent/50 bg-accent/[0.06]'
                    : 'border-white/[0.06] hover:border-white/[0.12] hover:bg-white/[0.03]'
                } ${locked ? 'cursor-pointer opacity-60 hover:opacity-100' : ''}`}
              >
                <span
                  className="h-9 w-9 flex-shrink-0 rounded-md ring-1 ring-inset ring-white/10"
                  style={opt.swatch}
                />
                <span className="flex-1 text-sm font-medium text-textPrimary">{opt.name}</span>
                {locked && (
                  <span className="inline-flex items-center gap-1 rounded-full border border-amber-400/30 bg-amber-400/10 px-2 py-0.5 text-[9px] font-semibold uppercase tracking-wide text-amber-300/90">
                    Get {opt.tier === 'subscriber' ? 'Subscriber' : 'Supporter'}
                    <ExternalLink size={9} className="opacity-70" />
                  </span>
                )}
                {selected && <Check size={16} className="text-accent" />}
              </button>
            );
          })}
        </div>
      </div>

      {/* StreamNook Atmospheres: our own profile cosmetic that decorates BOTH the
          profile background AND chat messages (the whole package, unlike a paint
          which is only a background). A subscriber perk. Click the active one to
          remove it (back to the tier aura). */}
      <div className="settings-card p-4">
        <div className="mb-1.5 flex items-center gap-1.5">
          <img src={streamNookLogo} alt="" className="h-4 w-4 object-contain" draggable={false} />
          <h4 className="text-sm font-semibold uppercase tracking-wide text-textPrimary">Atmospheres</h4>
          {!canAtmosphere && (
            <button
              type="button"
              onClick={() => openSupportFor('subscriber')}
              className="inline-flex items-center gap-1 rounded-full border border-amber-400/30 bg-amber-400/10 px-2 py-0.5 text-[9px] font-semibold uppercase tracking-wide text-amber-300/90 transition-colors hover:bg-amber-400/20"
            >
              Get Subscriber
              <ExternalLink size={9} className="opacity-70" />
            </button>
          )}
        </div>
        <p className="text-[12px] leading-relaxed text-textSecondary">
          A subscriber perk, and the whole package: an Atmosphere themes your profile AND decorates
          your chat messages with the same animated look. Tap the active one to remove it.
        </p>
        <div className="mt-3 space-y-2">
          {listAtmospheres()
            // Cologne renders as its own card with add-on toggles, below, once
            // earned. Before that it lists here as a locked accolade row.
            .filter((a) => !(isCologneTheme(a.id) && earnedAccolades.has(MAJOR_COLOGNE_ACCOLADE_ID)))
            .map((a) => {
            const selected = profileTheme === a.id;
            const locked = !atmUnlocked(a);
            // Accolade-gated Atmospheres show locked until earned so the whole
            // collection is visible. A secret challenge keeps its method to
            // itself ("Hidden challenge"); a public one names the accolade.
            // Neither is a purchase, so the row is inert rather than a link.
            const unlock = getAtmosphereUnlock(a);
            const lockedAccolade = locked && unlock.kind === 'accolade';
            const tip = lockedAccolade ? unlock.label : (atmosphereUnlockNote(a) ?? a.name);
            return (
              <Tooltip key={a.id} content={tip} side="top">
              <button
                type="button"
                aria-disabled={lockedAccolade || undefined}
                onClick={() =>
                  lockedAccolade
                    ? undefined
                    : locked
                      ? openSupportFor('subscriber')
                      : selected
                        ? selectProfileTheme('tier', 'free')
                        : selectProfileTheme(a.id, 'subscriber', a.unlock?.kind === 'accolade')
                }
                onMouseEnter={() => setPreviewThemeId(a.id)}
                onMouseLeave={() => setPreviewThemeId(null)}
                className={`flex w-full items-center gap-3 rounded-lg border p-2.5 text-left transition-colors ${
                  selected
                    ? 'border-accent/50 bg-accent/[0.06]'
                    : lockedAccolade
                      ? 'border-dashed border-white/[0.08]'
                      : 'border-white/[0.06] hover:border-white/[0.12] hover:bg-white/[0.03]'
                } ${lockedAccolade ? 'cursor-default opacity-60' : locked ? 'cursor-pointer opacity-60 hover:opacity-100' : ''}`}
              >
                <span
                  className={`h-9 w-14 flex-shrink-0 rounded-md ring-1 ring-inset ring-white/10 ${lockedAccolade ? 'saturate-[0.35]' : ''}`}
                  style={{ background: a.swatch }}
                />
                <span className="flex-1 text-sm font-medium text-textPrimary">{a.name}</span>
                {lockedAccolade && (
                  <span className="inline-flex items-center gap-1 rounded-full border border-amber-400/30 bg-amber-400/[0.06] px-2 py-0.5 text-[9px] font-semibold uppercase tracking-wide text-amber-300/90">
                    <Lock size={9} />
                    {unlock.hidden ? 'Hidden challenge' : 'Accolade'}
                  </span>
                )}
                {selected && <span className="text-[10px] font-medium text-textMuted">Active</span>}
                {selected && <Check size={16} className="text-accent" />}
              </button>
              </Tooltip>
            );
          })}
          {/* CS2 Major Cologne: one earned look whose coin + border are opt-in
              add-ons gated by support tier (supporter = coin, subscriber = both). */}
          {earnedAccolades.has(MAJOR_COLOGNE_ACCOLADE_ID) && (() => {
            const applied = parseCologneTheme(profileTheme);
            const active = !!applied;
            const coinOn = applied?.coin ?? false;
            const frameOn = applied?.frame ?? false;
            // The Cologne def (R2 asset URLs); null until the catalog loads. The
            // swatch renders the live animated glass wash, not a static image.
            const cologneAtm = getAtmosphere(MAJOR_COLOGNE_THEME_ID);
            return (
              <div className="overflow-hidden rounded-lg border border-white/[0.06]">
                <button
                  type="button"
                  onClick={() => (active ? selectProfileTheme('tier', 'free') : selectProfileTheme(MAJOR_COLOGNE_THEME_ID, 'free', true))}
                  onMouseEnter={() => setPreviewThemeId(active ? profileTheme : MAJOR_COLOGNE_THEME_ID)}
                  onMouseLeave={() => setPreviewThemeId(null)}
                  className={`flex w-full items-center gap-3 p-2.5 text-left transition-colors ${
                    active ? 'bg-accent/[0.06]' : 'hover:bg-white/[0.03]'
                  }`}
                >
                  <span
                    className="relative h-9 w-14 flex-shrink-0 overflow-hidden rounded-md ring-1 ring-inset ring-white/10"
                    style={{ background: cologneAtm?.baseColor }}
                  >
                    {cologneAtm?.chromeTexture && (
                      <MajorCologneChrome textureUrl={cologneAtm.chromeTexture} bare />
                    )}
                  </span>
                  <span className="flex-1 text-sm font-medium text-textPrimary">CS2 Major Cologne 2026</span>
                  {active && <span className="text-[10px] font-medium text-textMuted">Active</span>}
                  {active && <Check size={16} className="text-accent" />}
                </button>
                {active && (
                  <div className="space-y-0.5 border-t border-white/[0.06] bg-black/20 px-3 py-1.5">
                    {([
                      // tierMet('supporter') is true for subscribers too (they're
                      // the higher tier), so a subscriber always gets the coin.
                      { label: 'Major Medallion', on: coinOn, can: tierMet('supporter'), tier: 'supporter' as const, next: { coin: !coinOn, frame: frameOn } },
                      { label: 'Gilded Plaque', on: frameOn, can: tierMet('subscriber'), tier: 'subscriber' as const, next: { coin: coinOn, frame: !frameOn } },
                    ]).map((opt) => (
                      <div key={opt.label} className="flex items-center justify-between gap-3 py-1">
                        <span className="text-[13px] text-textPrimary">{opt.label}</span>
                        {opt.can ? (
                          <button
                            type="button"
                            role="switch"
                            aria-checked={opt.on}
                            aria-label={`${opt.on ? 'Disable' : 'Enable'} ${opt.label.toLowerCase()}`}
                            onClick={() => selectProfileTheme(buildCologneTheme(opt.next), 'free', true)}
                            className={`relative inline-flex h-5 w-9 flex-shrink-0 items-center rounded-full transition-colors ${
                              opt.on ? 'bg-accent' : 'bg-white/[0.12]'
                            }`}
                          >
                            <span
                              className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white transition-transform ${
                                opt.on ? 'translate-x-4' : 'translate-x-1'
                              }`}
                            />
                          </button>
                        ) : (
                          <button
                            type="button"
                            onClick={() => openSupportFor(opt.tier)}
                            className="inline-flex items-center gap-1 text-[11px] font-medium text-textSecondary transition-colors hover:text-accent"
                          >
                            {opt.tier === 'subscriber' ? 'Subscriber' : 'Supporter'}
                            <ExternalLink size={9} className="opacity-70" />
                          </button>
                        )}
                      </div>
                    ))}
                  </div>
                )}
              </div>
            );
          })()}
        </div>
      </div>

      {/* Profile visibility: per-section hide/unhide for the PUBLIC profile (what
          other StreamNook users see in the overlay). Self view always shows all. */}
      <div className="settings-card p-4">
        <h4 className="text-sm font-semibold text-textPrimary">Profile visibility</h4>
        <p className="mt-0.5 text-[12px] leading-relaxed text-textSecondary">
          Choose what shows on your public profile (what other StreamNook users see when they open
          it). Your subscriptions and spend are always private.
        </p>
        <div className="mt-3 space-y-0.5">
          {VISIBILITY_SECTIONS.map((s) => {
            const visible = !hiddenSecs.includes(s.key);
            return (
              <div key={s.key} className="flex items-center justify-between gap-3 px-1 py-1.5">
                <span className="text-sm text-textPrimary">{s.label}</span>
                <button
                  type="button"
                  role="switch"
                  aria-checked={visible}
                  onClick={() => toggleSectionVisibility(s.key)}
                  className={`relative inline-flex h-5 w-9 flex-shrink-0 items-center rounded-full transition-colors ${
                    visible ? 'bg-accent' : 'bg-white/[0.12]'
                  }`}
                  aria-label={`${visible ? 'Hide' : 'Show'} ${s.label}`}
                >
                  <span
                    className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                      visible ? 'translate-x-[18px]' : 'translate-x-0.5'
                    }`}
                  />
                </button>
              </div>
            );
          })}
        </div>
      </div>

      <div className="settings-card p-4">
        <div className="flex items-center justify-between mb-4">
          <div className="flex items-center gap-1.5">
            <Tooltip content="Open your Twitch profile" side="top">
              <button
                onClick={handleOpenTwitchProfile}
                className="cursor-pointer hover:scale-110 transition-transform"
                aria-label="Open your Twitch profile"
              >
                <svg fill="currentColor" viewBox="0 0 512 512" className="w-4 h-4 text-[#9146FF]">
                  <path d="M80,32,48,112V416h96v64h64l64-64h80L464,304V32ZM416,288l-64,64H256l-64,64V352H112V80H416Z" />
                  <rect x="320" y="143" width="48" height="129" />
                  <rect x="208" y="143" width="48" height="129" />
                </svg>
              </button>
            </Tooltip>
            <h4 className="text-sm font-semibold text-textPrimary uppercase tracking-wide">
              Global Cosmetics
            </h4>
          </div>
          <button
            onClick={() => fetchChatIdentity(true)}
            disabled={isFetchingIdentity}
            className="p-1.5 text-textSecondary hover:text-accent hover:bg-glass rounded transition-all"
          >
            <svg
              width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor"
              strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"
              className={isFetchingIdentity ? 'animate-spin' : ''}
            >
              <path d="M21 12a9 9 0 0 0-9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
              <path d="M3 3v5h5" />
              <path d="M3 12a9 9 0 0 0 9 9 9.75 9.75 0 0 0 6.74-2.74L21 16" />
              <path d="M16 21h5v-5" />
            </svg>
          </button>
        </div>

        {chatIdentityBadges.length > 0 ? (
          <div className="grid grid-cols-8 gap-2">
            {chatIdentityBadges.map((badge) => (
              <Tooltip key={`${badge.id}-${badge.version}`} content={badge.title} side="top">
                <div
                  className={`
                    relative p-2 rounded-lg cursor-pointer transition-all flex items-center justify-center
                    ${badge.is_selected ? 'glass-input border-accent/50 ring-1 ring-accent/30' : 'border border-transparent hover:bg-glass hover:border-borderLight'}
                    ${updatingBadgeId === badge.id ? 'opacity-50 cursor-wait' : ''}
                  `}
                  onClick={() => !updatingBadgeId && updateChatIdentity(badge)}
                >
                  <img src={badge.image_url} alt={badge.title} className="w-8 h-8" />
                  {badge.is_selected && <CheckChip tone="accent" />}
                </div>
              </Tooltip>
            ))}
          </div>
        ) : (
          <p className="text-textSecondary text-sm italic">
            {isFetchingIdentity ? 'Loading badges...' : 'No badges available'}
          </p>
        )}
      </div>

      {(allSeventvPaints.length > 0 || seventvBadges.length > 0) && (
        <div className="settings-card p-4">
          <div className="flex items-center justify-between mb-4">
            <div className="flex items-center gap-3">
              <div className="flex items-center gap-1.5">
                <Tooltip content="Edit your 7TV cosmetics" side="top">
                  <button
                    onClick={handleOpen7TVCosmetics}
                    className="cursor-pointer hover:scale-110 transition-transform"
                    aria-label="Edit your 7TV cosmetics"
                  >
                    <svg className="w-4 h-4 text-[#29b6f6]" viewBox="0 0 28 21" fill="currentColor">
                      <path d="M20.7465 5.48825L21.9799 3.33745L22.646 2.20024L21.4125 0.0494437V0H14.8259L17.2928 4.3016L17.9836 5.48825H20.7465Z" />
                      <path d="M7.15395 19.9258L14.5546 7.02104L15.4673 5.43884L13.0004 1.13724L12.3097 0.0247596H1.8995L0.666057 2.17556L0 3.31276L1.23344 5.46356V5.51301H9.12745L2.96025 16.267L2.09685 17.7998L3.33029 19.9506V20H7.15395" />
                      <path d="M17.4655 19.9257H21.2398L26.1736 11.3225L27.037 9.83924L25.8036 7.68844V7.63899H22.0046L19.5377 11.9406L19.365 12.262L16.8981 7.96038L16.7255 7.63899L14.2586 11.9406L13.5679 13.1272L17.2682 19.5796L17.4655 19.9257Z" />
                    </svg>
                  </button>
                </Tooltip>
                <h4 className="text-sm font-semibold text-textPrimary uppercase tracking-wide">
                  Cosmetics
                </h4>
              </div>
              {seventvAuthConnected && (
                <span className="px-2 py-0.5 text-[10px] font-medium bg-green-500/20 text-green-400 rounded-full">
                  Connected
                </span>
              )}
            </div>

            {seventvUserId && (
              seventvAuthConnected ? (
                <button
                  onClick={async () => {
                    await invoke('logout_seventv');
                    if (mountedRef.current) setSeventvAuthConnected(false);
                  }}
                  className="px-3 py-1.5 text-xs font-medium text-red-400 hover:bg-red-500/10 rounded-lg transition-all flex items-center gap-1.5"
                >
                  <Unlink size={14} />
                  Disconnect
                </button>
              ) : (
                <button
                  onClick={async () => {
                    setIsConnecting7TV(true);
                    await invoke('open_seventv_login_window');
                  }}
                  disabled={isConnecting7TV}
                  className="px-3 py-1.5 text-xs font-medium text-[#29b6f6] hover:bg-[#29b6f6]/10 rounded-lg transition-all flex items-center gap-1.5"
                >
                  <Link size={14} />
                  {isConnecting7TV ? 'Connecting...' : 'Connect to Edit'}
                </button>
              )
            )}
          </div>

          {allSeventvPaints.length > 0 && (
            <div className="mb-4">
              <p className="text-xs text-textSecondary mb-2 font-medium">Paints</p>
              <div className="flex flex-wrap gap-2">
                {allSeventvPaints.map((paint) => {
                  const isSelected = seventvPaint?.id === paint.id;
                  const isUpdating = updatingSeventvPaintId === paint.id;
                  return (
                    <Tooltip key={paint.id} content={seventvAuthConnected ? paint.name : `${paint.name} - Connect to edit`} side="top">
                      <div
                        className={`
                          relative px-3 py-1.5 rounded-lg cursor-pointer transition-all text-sm font-bold
                          ${isSelected ? 'glass-input border-[#29b6f6]/50 ring-1 ring-[#29b6f6]/30' : 'border border-transparent hover:bg-glass hover:border-borderLight'}
                          ${isUpdating ? 'opacity-50' : ''}
                          ${!seventvAuthConnected ? 'cursor-default' : ''}
                        `}
                        onClick={() => seventvAuthConnected && !isUpdating && handleSelectSeventvPaint(isSelected ? null : paint)}
                      >
                        <span style={computePaintStyle(paint as any, '#29b6f6')}>{paint.name}</span>
                        {isSelected && <CheckChip tone="seventv" />}
                      </div>
                    </Tooltip>
                  );
                })}
              </div>
            </div>
          )}

          {seventvBadges.length > 0 && (
            <div>
              <p className="text-xs text-textSecondary mb-2 font-medium">Badges</p>
              <div className="flex flex-wrap gap-2">
                {seventvBadges.map((badge, idx) => {
                  const urls = getBadgeImageUrls(badge as any);
                  const isSelected = (badge as any).selected;
                  const isUpdating = updatingSeventvBadgeId === badge.id;
                  return urls.url4x ? (
                    <Tooltip key={`${badge.id}-${idx}`} content={badge.tooltip || badge.name} side="top">
                      <div
                        className={`
                          relative p-2 rounded-lg cursor-pointer transition-all
                          ${isSelected ? 'glass-input border-[#29b6f6]/50 ring-1 ring-[#29b6f6]/30' : 'border border-transparent hover:bg-glass hover:border-borderLight'}
                          ${isUpdating ? 'opacity-50' : ''}
                          ${!seventvAuthConnected ? 'cursor-default' : ''}
                        `}
                        onClick={() => seventvAuthConnected && !isUpdating && handleSelectSeventvBadge(isSelected ? null : badge)}
                      >
                        <FallbackImage
                          src={urls.url4x}
                          fallbackUrls={getBadgeFallbackUrls(badge.id).slice(1)}
                          alt={badge.tooltip || badge.name}
                          className="w-8 h-8"
                        />
                        {isSelected && <CheckChip tone="seventv" />}
                      </div>
                    </Tooltip>
                  ) : null;
                })}
              </div>
            </div>
          )}
        </div>
      )}

      {allThirdParty.length > 0 && (
        <div className="settings-card p-4">
          <h4 className="text-sm font-semibold text-textPrimary uppercase tracking-wide mb-4">
            Other Badges
          </h4>
          <div className="flex flex-wrap gap-2">
            {allThirdParty.map((badge: any, idx) => {
              const shown = isThirdPartyShown(badge);
              return (
                <Tooltip key={`${badge.provider}-${badge.id}-${idx}`} content={`${badge.title} (${badge.provider.toUpperCase()})`} side="top">
                  <div
                    onClick={() => toggleThirdParty(badge)}
                    className={`
                      relative p-2 rounded-lg cursor-pointer transition-all
                      ${shown
                        ? 'glass-input border-accent/50 ring-1 ring-accent/30'
                        : 'border border-transparent opacity-50 hover:opacity-80 hover:bg-glass hover:border-borderLight'}
                    `}
                  >
                    <img src={badge.image4x || badge.imageUrl} alt={badge.title} className="w-8 h-8" />
                    {shown && <CheckChip tone="accent" />}
                  </div>
                </Tooltip>
              );
            })}
          </div>
        </div>
      )}

        </>
      )}
    </div>
  );
};

export default ProfileSettings;
