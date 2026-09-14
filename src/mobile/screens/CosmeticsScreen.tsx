// Cosmetics equip: your StreamNook badge and Atmosphere, same server writes
// the desktop editor uses (setActiveCosmetic / setProfileTheme). Rendering
// reuses the shared catalog + asset resolution, so what you equip here paints
// identically in chat and on your profile everywhere.
//
// 7TV paints and badges sit alongside them, equipped through the same
// set_seventv_paint / set_seventv_badge commands the desktop editor calls. What
// you own is public, so it lists whether or not you have signed in to 7TV;
// signing in is only needed to change what is worn.
//
// Twitch's own global badge is equippable too, through the same GQL mutation
// the desktop editor uses. Desktop wraps that in a webview fallback for when
// GQL declines; the phone has no second window, so it exposes the GQL half on
// its own and says plainly when it is unavailable. That path needs the Drops
// token, so without it the earned badges are still listed, just not selectable.
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { ArrowLeft, CaretRight, Check, Lock } from 'phosphor-react';
import { useAppStore } from '../../stores/AppStore';
import { useMobileNavStore } from '../navStore';
import { DrillInScreen } from '../ui/DrillInScreen';
import { FallbackImage } from '../../components/FallbackImage';
import { computePaintStyle, clearUserCache as clear7TVCache } from '../../services/seventvService';
import {
  applyLocalCosmeticSelection,
  getFullProfileWithFallback,
} from '../../services/cosmeticsCache';
import { normalizeProfileBadges, type NormalizedBadge } from '../../utils/profileBadges';
import { connectSevenTv, getSevenTvStatus } from '../cosmetics/sevenTvConnect';

/**
 * A named, countable section that opens on tap.
 *
 * An account can own dozens of paints and well over a hundred Twitch badges, so
 * laid out flat these would bury the StreamNook badge and Atmosphere under a
 * minute of scrolling. Collapsed, the screen opens exactly as it did before and
 * the count says what is inside without opening it. Same shape and timings as
 * the drops groups, so the two read as one idea.
 */
/** Mirrors the Rust `ChatIdentityBadge`. `version` is what the mutation needs. */
interface ChatIdentityBadge {
  id: string;
  version: string;
  title: string;
  image_url: string;
  is_selected: boolean;
}

const CollapsibleSection: React.FC<{
  title: string;
  count: number;
  expanded: boolean;
  onToggle: () => void;
  children: React.ReactNode;
}> = ({ title, count, expanded, onToggle, children }) => (
  <div>
    <button
      onClick={onToggle}
      aria-expanded={expanded}
      className="w-full flex items-center gap-2 mt-4 mb-1.5 text-left active:opacity-70 transition-opacity"
    >
      <span className="text-[12px] font-semibold text-textMuted uppercase tracking-wide">
        {title}
      </span>
      <span className="text-[12px] text-textMuted">{count}</span>
      <motion.span
        className="ml-auto shrink-0 flex text-textMuted"
        animate={{ rotate: expanded ? 90 : 0 }}
        transition={{ duration: 0.22, ease: [0.16, 1, 0.3, 1] }}
      >
        <CaretRight size={14} weight="bold" />
      </motion.span>
    </button>
    <AnimatePresence>
      {expanded && (
        <motion.div
          initial={{ height: 0, opacity: 0 }}
          animate={{ height: 'auto', opacity: 1 }}
          exit={{ height: 0, opacity: 0 }}
          transition={{ duration: 0.26, ease: [0.16, 1, 0.3, 1] }}
          className="overflow-hidden"
        >
          <div className="pb-1">{children}</div>
        </motion.div>
      )}
    </AnimatePresence>
  </div>
);
import {
  getAccolades,
  getActiveCosmeticSlug,
  getAllCosmetics,
  getOwnedCosmeticSlugs,
  getProfilePrefs,
  setActiveCosmetic,
  setProfileTheme,
} from '../../services/supabaseService';
import { listAtmospheres, type Atmosphere } from '../../services/atmospheres';
import { isSubscriber } from '../../services/subscriberService';
import { resolveCosmeticAsset } from '../../components/cosmeticAssets';
import { Logger } from '../../utils/logger';

export const CosmeticsScreen: React.FC = () => {
  const open = useMobileNavStore((s) => s.cosmeticsOpen);
  const setCosmeticsOpen = useMobileNavStore((s) => s.setCosmeticsOpen);
  const currentUser = useAppStore((s) => s.currentUser);
  const addToast = useAppStore((s) => s.addToast);

  const userId = currentUser?.user_id ?? '';
  const [ownedSlugs, setOwnedSlugs] = useState<Set<string>>(new Set());
  const [activeBadge, setActiveBadge] = useState<string | null>(null);
  const [profileTheme, setProfileThemeState] = useState<string>('tier');
  const [subscribed, setSubscribed] = useState(false);
  const [earnedAccolades, setEarnedAccolades] = useState<Set<string>>(new Set());

  // 7TV and Twitch. Both come out of one profile read, which is already the
  // path the mobile profile sheet uses, so nothing new is being fetched.
  const [sevenTvConnected, setSevenTvConnected] = useState(false);
  const [sevenTvUserId, setSevenTvUserId] = useState<string | null>(null);
  const [paints, setPaints] = useState<any[]>([]);
  const [sevenTvBadges, setSevenTvBadges] = useState<any[]>([]);
  const [activePaintId, setActivePaintId] = useState<string | null>(null);
  const [activeSevenTvBadgeId, setActiveSevenTvBadgeId] = useState<string | null>(null);
  const [twitchBadges, setTwitchBadges] = useState<NormalizedBadge[]>([]);
  const [identityBadges, setIdentityBadges] = useState<ChatIdentityBadge[]>([]);
  const [dropsAuthed, setDropsAuthed] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const [openSections, setOpenSections] = useState<Set<string>>(new Set());

  const refresh = useCallback(async () => {
    if (!userId) return;
    // Gather everything first, then commit state once after the awaits, so no
    // setState lands synchronously inside the calling effect.
    const owned = getOwnedCosmeticSlugs(userId);
    const active = getActiveCosmeticSlug(userId);
    let theme = 'tier';
    try {
      theme = (await getProfilePrefs(userId)).profileTheme || 'tier';
    } catch (err) {
      Logger.warn('[Cosmetics] prefs read failed:', err);
    }
    let sub = false;
    try {
      sub = await isSubscriber(userId);
    } catch {
      sub = false;
    }
    // Accolade-gated atmospheres unlock off user_accolades, NOT owned cosmetic
    // slugs, and stay hidden until earned (the unlock is meant to be a
    // surprise). Same rule the desktop cosmetics panel uses.
    let accolades = new Set<string>();
    try {
      accolades = new Set(await getAccolades(userId));
    } catch {
      /* leave empty; accolade atmospheres simply stay hidden */
    }
    setOwnedSlugs(owned);
    setActiveBadge(active);
    setProfileThemeState(theme);
    setSubscribed(sub);
    setEarnedAccolades(accolades);
  }, [userId]);

  const username = currentUser?.login || currentUser?.username || '';

  /** Applies a freshly-read 7TV cosmetics set, selection included. */
  const applySevenTv = useCallback((cosmetics: { paints: any[]; badges: any[]; seventvUserId?: string }) => {
    setPaints(cosmetics.paints ?? []);
    setSevenTvBadges(cosmetics.badges ?? []);
    setActivePaintId((cosmetics.paints ?? []).find((p) => p?.selected)?.id ?? null);
    setActiveSevenTvBadgeId((cosmetics.badges ?? []).find((b) => b?.selected)?.id ?? null);
    if (cosmetics.seventvUserId) setSevenTvUserId(cosmetics.seventvUserId);
  }, []);

  const loadProfileCosmetics = useCallback(async () => {
    if (!userId) return;
    // Whether 7TV is signed in is independent of what is owned: the catalog is
    // public, the token only authorises changing the selection.
    try {
      const status = await getSevenTvStatus();
      setSevenTvConnected(status.is_authenticated);
      if (status.user_id) setSevenTvUserId(status.user_id);
    } catch (err) {
      Logger.debug('[Cosmetics] 7TV status unavailable:', err);
    }
    try {
      const profile = await getFullProfileWithFallback(userId, username);
      applySevenTv(profile.seventvCosmetics);
      setTwitchBadges(normalizeProfileBadges({ cachedProfile: profile }).twitch);
    } catch (err) {
      Logger.debug('[Cosmetics] profile cosmetics unavailable:', err);
    }
    // The selectable Twitch list is a different set from the earned one: global
    // badges only, carrying the version the mutation needs and which is worn.
    // It rides the Drops token, so there is nothing to ask for without it.
    try {
      const authed = await invoke<boolean>('is_drops_authenticated');
      setDropsAuthed(authed);
      if (authed && username) {
        setIdentityBadges(
          await invoke<ChatIdentityBadge[]>('fetch_chat_identity_badges_gql', { username }),
        );
      }
    } catch (err) {
      Logger.debug('[Cosmetics] Twitch badge list unavailable:', err);
    }
  }, [userId, username, applySevenTv]);

  useEffect(() => {
    // Load-on-open sync from external stores (Supabase registries + prefs,
    // then 7TV and Twitch). Every state commit happens after a real await.
    if (!open) return;
    void refresh();
    void loadProfileCosmetics();
  }, [open, refresh, loadProfileCosmetics]);

  // No early return: AnimatePresence inside DrillInScreen needs the subtree to
  // stay mounted for the length of the exit, and a component that returns null
  // when closed can never animate away.

  const badges = getAllCosmetics().filter(
    (c) => (c.kind ?? 'badge') === 'badge' && ownedSlugs.has(c.slug),
  );
  const atmospheres = listAtmospheres();

  // Reuses the same image resolution the profile sheet renders other people's
  // 7TV badges with, rather than a second guess at 7TV's CDN layout. Ids come
  // through unchanged, so the raw list's `selected` still lines up.
  const sevenTvBadgeTiles = useMemo(
    () =>
      normalizeProfileBadges({
        cachedProfile: { seventvCosmetics: { paints: [], badges: sevenTvBadges } } as never,
      }).seventv.filter((b) => !!b.src),
    [sevenTvBadges],
  );

  const isOpen = (key: string) => openSections.has(key);
  const toggleSection = (key: string) =>
    setOpenSections((prev) => {
      const next = new Set(prev);
      if (!next.delete(key)) next.add(key);
      return next;
    });

  const equipBadge = async (slug: string) => {
    const next = activeBadge === slug ? null : slug;
    const prev = activeBadge;
    setActiveBadge(next);
    const res = await setActiveCosmetic(userId, next);
    if (!res.ok) {
      setActiveBadge(prev);
      addToast(res.error || 'Could not equip that badge.', 'error');
    }
  };

  // Mirrors the desktop rule: accolade atmospheres unlock for ANY member who
  // earned the accolade (no subscription); subscriber atmospheres are owned
  // per-item, so lapsing keeps everything already unlocked.
  const atmosphereAllowed = (atm: Atmosphere): boolean =>
    atm.unlock?.kind === 'accolade'
      ? earnedAccolades.has(atm.unlock.accoladeId)
      : ownedSlugs.has(atm.id) || subscribed;

  // Accolade-gated atmospheres stay hidden until earned. Event atmospheres
  // (Cologne) ARE included: excluding them here is what hid the CS2 one from
  // the picker entirely, so an owner could not equip it at all.
  const visibleAtmospheres = atmospheres.filter(
    (a) => a.unlock?.kind !== 'accolade' || earnedAccolades.has(a.unlock.accoladeId),
  );

  const equipTheme = async (theme: string) => {
    const prev = profileTheme;
    setProfileThemeState(theme);
    try {
      await setProfileTheme(userId, theme);
    } catch (err) {
      Logger.error('[Cosmetics] theme write failed:', err);
      setProfileThemeState(prev);
      addToast('Could not apply that atmosphere.', 'error');
    }
  };

  /**
   * Runs `apply` with the 7TV user id, signing in first if that has not
   * happened yet.
   *
   * Owned cosmetics list without a token, so the first tap on a tile is the
   * natural place for sign-in rather than a separate button to find first.
   */
  const withSevenTvAuth = async (apply: (sevenTvId: string) => Promise<void>) => {
    let uid = sevenTvUserId;
    if (!sevenTvConnected) {
      if (connecting) return;
      setConnecting(true);
      try {
        if (!(await connectSevenTv(userId))) return;
        const status = await getSevenTvStatus();
        setSevenTvConnected(status.is_authenticated);
        if (status.user_id) {
          uid = status.user_id;
          setSevenTvUserId(status.user_id);
        }
        if (!status.is_authenticated) return;
      } catch (err) {
        Logger.error('[Cosmetics] 7TV sign-in failed:', err);
        addToast('Could not sign in to 7TV.', 'error');
        return;
      } finally {
        setConnecting(false);
      }
    }
    if (!uid) {
      addToast('7TV did not identify your account.', 'error');
      return;
    }
    await apply(uid);
  };

  /**
   * Wear a Twitch global badge.
   *
   * There is no unequip: Twitch's editor only ever swaps which badge is worn,
   * so tapping the one already on is a no-op rather than a way to clear it.
   */
  const equipTwitchBadge = async (badge: ChatIdentityBadge) => {
    if (badge.is_selected) return;
    const prev = identityBadges;
    setIdentityBadges((all) => all.map((b) => ({ ...b, is_selected: b.id === badge.id })));
    try {
      await invoke('update_chat_identity_gql', {
        badgeId: badge.id,
        badgeVersion: badge.version,
      });
    } catch (err) {
      Logger.error('[Cosmetics] Twitch badge write failed:', err);
      setIdentityBadges(prev);
      const msg = err instanceof Error ? err.message : String(err ?? '');
      addToast(
        msg.includes('NEEDS_DROPS_AUTH')
          ? 'Sign in to Drops to change your Twitch badge.'
          : 'Could not equip that badge.',
        'error',
      );
    }
  };

  /** Sign in on its own, so connecting is not something you only stumble into. */
  const signInSevenTv = () => withSevenTvAuth(async () => {});

  const disconnectSevenTv = async () => {
    try {
      await invoke('logout_seventv');
      setSevenTvConnected(false);
    } catch (err) {
      Logger.error('[Cosmetics] 7TV sign-out failed:', err);
      addToast('Could not sign out of 7TV.', 'error');
    }
  };

  /**
   * A rejected write usually means the token lapsed, and the only way to find
   * out is to try. Dropping the connected flag sends the next tap back through
   * sign-in instead of failing the same way forever.
   */
  const handleSevenTvWriteError = (err: unknown, what: string) => {
    Logger.error(`[Cosmetics] 7TV ${what} write failed:`, err);
    const msg = err instanceof Error ? err.message : String(err ?? '');
    if (msg.includes('SESSION_EXPIRED')) {
      setSevenTvConnected(false);
      addToast('Your 7TV sign-in expired. Tap again to sign back in.', 'error');
    } else {
      addToast(`Could not equip that ${what}.`, 'error');
    }
  };

  // Both writes flip the selection locally rather than re-reading it. 7TV's
  // read API lags its own mutation by a few seconds, so a refetch here would
  // cache the PREVIOUS pick and the change would not show until a reload. The
  // desktop editor settled on the same approach for the same reason. Clearing
  // the lower-level 7TV cache keeps a later natural fetch fresh.
  const equipPaint = (paintId: string) =>
    withSevenTvAuth(async (sevenTvId) => {
      const next = activePaintId === paintId ? null : paintId;
      const prev = activePaintId;
      setActivePaintId(next);
      try {
        const res = await invoke<{ success: boolean }>('set_seventv_paint', {
          userId: sevenTvId,
          paintId: next,
        });
        if (!res.success) throw new Error('7TV rejected the paint change');
        setPaints((all) => all.map((p) => ({ ...p, selected: p.id === next })));
        clear7TVCache();
        applyLocalCosmeticSelection(userId, { paintId: next });
      } catch (err) {
        setActivePaintId(prev);
        handleSevenTvWriteError(err, 'paint');
      }
    });

  const equipSevenTvBadge = (badgeId: string) =>
    withSevenTvAuth(async (sevenTvId) => {
      const next = activeSevenTvBadgeId === badgeId ? null : badgeId;
      const prev = activeSevenTvBadgeId;
      setActiveSevenTvBadgeId(next);
      try {
        const res = await invoke<{ success: boolean }>('set_seventv_badge', {
          userId: sevenTvId,
          badgeId: next,
        });
        if (!res.success) throw new Error('7TV rejected the badge change');
        setSevenTvBadges((all) => all.map((b) => ({ ...b, selected: b.id === next })));
        clear7TVCache();
        applyLocalCosmeticSelection(userId, { badgeId: next });
      } catch (err) {
        setActiveSevenTvBadgeId(prev);
        handleSevenTvWriteError(err, 'badge');
      }
    });

  return (
    <DrillInScreen
      open={open}
      layerKey="cosmetics"
      className="absolute inset-0 z-50 bg-background flex flex-col"
      style={{ paddingTop: 'var(--sn-safe-t, 0px)' }}
    >
      <div className="flex items-center gap-1 px-2 py-2 shrink-0 border-b border-borderSubtle">
        <button
          onClick={() => setCosmeticsOpen(false)}
          className="sn-touch flex items-center justify-center text-textSecondary"
          aria-label="Back"
        >
          <ArrowLeft size={22} />
        </button>
        <h1 className="text-lg font-bold text-textPrimary">Cosmetics</h1>
      </div>
      <div
        className="flex-1 min-h-0 overflow-y-auto px-4 py-3"
        style={{ paddingBottom: 'calc(var(--sn-safe-b, 0px) + 16px)' }}
      >
        <div className="text-[12px] font-semibold text-textMuted uppercase tracking-wide mb-1.5">
          Badge
        </div>
        {badges.length === 0 ? (
          <div className="glass-panel p-4 text-[13px] text-textMuted mb-4">
            No badges owned yet. Earned and event badges land here.
          </div>
        ) : (
          <div className="grid grid-cols-4 sm:grid-cols-6 lg:grid-cols-8 gap-2 mb-4">
            {badges.map((badge) => {
              const asset = resolveCosmeticAsset(badge);
              const isActive = activeBadge === badge.slug;
              return (
                <button
                  key={badge.slug}
                  onClick={() => void equipBadge(badge.slug)}
                  className={`relative glass-panel p-2 flex flex-col items-center gap-1.5 active:opacity-80 ${
                    isActive ? 'ring-2 ring-accent' : ''
                  }`}
                >
                  {asset ? (
                    <img src={asset} alt="" className="w-9 h-9 object-contain" draggable={false} />
                  ) : (
                    <div className="w-9 h-9 rounded bg-surface" />
                  )}
                  <span className="text-[10.5px] text-textSecondary text-center line-clamp-2 leading-tight">
                    {badge.name}
                  </span>
                  {isActive && (
                    <span className="absolute top-1 right-1 text-accent">
                      <Check size={13} weight="bold" />
                    </span>
                  )}
                </button>
              );
            })}
          </div>
        )}

        <div className="text-[12px] font-semibold text-textMuted uppercase tracking-wide mb-1.5">
          Atmosphere
        </div>
        <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 gap-2">
          <button
            onClick={() => void equipTheme('tier')}
            className={`glass-panel p-2.5 text-left active:opacity-80 ${
              profileTheme === 'tier' ? 'ring-2 ring-accent' : ''
            }`}
          >
            <div className="w-full h-12 rounded bg-surface mb-1.5" />
            <div className="flex items-center justify-between">
              <span className="text-[12.5px] text-textPrimary">Default</span>
              {profileTheme === 'tier' && <Check size={13} weight="bold" className="text-accent" />}
            </div>
          </button>
          {visibleAtmospheres.map((atm) => {
            const allowed = atmosphereAllowed(atm);
            const isActive = profileTheme === atm.id || profileTheme.startsWith(`${atm.id}+`);
            return (
              <button
                key={atm.id}
                onClick={() => allowed && void equipTheme(atm.id)}
                disabled={!allowed}
                className={`glass-panel p-2.5 text-left active:opacity-80 disabled:opacity-45 ${
                  isActive ? 'ring-2 ring-accent' : ''
                }`}
              >
                <div className="w-full h-12 rounded mb-1.5" style={{ background: atm.swatch }} />
                <div className="flex items-center justify-between">
                  <span className="text-[12.5px] text-textPrimary truncate">{atm.name}</span>
                  {isActive ? (
                    <Check size={13} weight="bold" className="text-accent shrink-0" />
                  ) : !allowed ? (
                    <Lock size={12} className="text-textMuted shrink-0" />
                  ) : null}
                </div>
              </button>
            );
          })}
        </div>

        {(paints.length > 0 || sevenTvBadgeTiles.length > 0) && (
          <div className="flex items-baseline gap-2 mt-5">
            <span className="text-[12px] font-semibold text-textMuted uppercase tracking-wide">
              7TV
            </span>
            <span className="text-[12px] text-textMuted">
              {sevenTvConnected ? 'Signed in' : 'Not signed in'}
            </span>
            <button
              onClick={() => (sevenTvConnected ? void disconnectSevenTv() : void signInSevenTv())}
              disabled={connecting}
              className="ml-auto text-[12.5px] text-accent active:opacity-70 disabled:opacity-50"
            >
              {connecting ? 'Signing in…' : sevenTvConnected ? 'Sign out' : 'Sign in'}
            </button>
          </div>
        )}

        {paints.length > 0 && (
          <CollapsibleSection
            title="7TV paints"
            count={paints.length}
            expanded={isOpen('paints')}
            onToggle={() => toggleSection('paints')}
          >
            <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 gap-2">
              {paints.map((paint) => {
                const isActive = activePaintId === paint.id;
                return (
                  <button
                    key={paint.id}
                    onClick={() => void equipPaint(paint.id)}
                    className={`glass-panel w-full p-2.5 flex items-center gap-1.5 text-left active:opacity-80 ${
                      isActive ? 'ring-2 ring-accent' : ''
                    }`}
                  >
                    {/* The paint IS the preview: rendering its own name in it is
                        the only way to see what it looks like. */}
                    <span
                      className="text-[14px] font-bold truncate leading-snug"
                      style={computePaintStyle(paint, '#9146FF')}
                    >
                      {paint.name}
                    </span>
                    {isActive && (
                      <Check size={13} weight="bold" className="text-accent shrink-0 ml-auto" />
                    )}
                  </button>
                );
              })}
            </div>
          </CollapsibleSection>
        )}

        {sevenTvBadgeTiles.length > 0 && (
          <CollapsibleSection
            title="7TV badges"
            count={sevenTvBadgeTiles.length}
            expanded={isOpen('seventvBadges')}
            onToggle={() => toggleSection('seventvBadges')}
          >
            <div className="grid grid-cols-4 sm:grid-cols-6 lg:grid-cols-8 gap-2">
              {sevenTvBadgeTiles.map((badge) => {
                const isActive = activeSevenTvBadgeId === badge.id;
                return (
                  <button
                    key={badge.id}
                    onClick={() => void equipSevenTvBadge(badge.id)}
                    className={`relative glass-panel w-full p-2 flex flex-col items-center gap-1.5 active:opacity-80 ${
                      isActive ? 'ring-2 ring-accent' : ''
                    }`}
                  >
                    <FallbackImage
                      src={badge.src as string}
                      fallbackUrls={badge.fallbackUrls}
                      srcSet={badge.srcSet}
                      alt={badge.title || ''}
                      className="w-9 h-9 object-contain"
                    />
                    <span className="text-[10.5px] text-textSecondary text-center line-clamp-2 leading-tight">
                      {badge.name || badge.title}
                    </span>
                    {isActive && (
                      <span className="absolute top-1 right-1 text-accent">
                        <Check size={13} weight="bold" />
                      </span>
                    )}
                  </button>
                );
              })}
            </div>
          </CollapsibleSection>
        )}

        {identityBadges.length > 0 ? (
          <CollapsibleSection
            title="Twitch badges"
            count={identityBadges.length}
            expanded={isOpen('twitch')}
            onToggle={() => toggleSection('twitch')}
          >
            <div className="grid grid-cols-4 sm:grid-cols-6 lg:grid-cols-8 gap-2">
              {identityBadges.map((badge) => (
                <button
                  key={`${badge.id}-${badge.version}`}
                  onClick={() => void equipTwitchBadge(badge)}
                  className={`relative glass-panel w-full p-2 flex flex-col items-center gap-1.5 active:opacity-80 ${
                    badge.is_selected ? 'ring-2 ring-accent' : ''
                  }`}
                >
                  <img
                    src={badge.image_url}
                    alt=""
                    draggable={false}
                    className="w-9 h-9 object-contain"
                  />
                  <span className="text-[10.5px] text-textSecondary text-center line-clamp-2 leading-tight">
                    {badge.title}
                  </span>
                  {badge.is_selected && (
                    <span className="absolute top-1 right-1 text-accent">
                      <Check size={13} weight="bold" />
                    </span>
                  )}
                </button>
              ))}
            </div>
          </CollapsibleSection>
        ) : (
          twitchBadges.length > 0 && (
            <CollapsibleSection
              title="Twitch badges"
              count={twitchBadges.length}
              expanded={isOpen('twitch')}
              onToggle={() => toggleSection('twitch')}
            >
              {/* Everything earned, but not selectable: choosing which one you
                  wear goes through Twitch's own editor, which needs the Drops
                  sign-in. */}
              <div className="glass-panel p-3">
                <div className="flex flex-wrap gap-2.5">
                  {twitchBadges.map((badge, i) => (
                    <FallbackImage
                      key={`${badge.id}-${i}`}
                      src={badge.src as string}
                      fallbackUrls={badge.fallbackUrls}
                      srcSet={badge.srcSet}
                      alt={badge.title || ''}
                      title={badge.title}
                      className="w-8 h-8 object-contain"
                    />
                  ))}
                </div>
                {!dropsAuthed && (
                  <div className="text-[11.5px] text-textMuted mt-2.5 leading-snug">
                    Sign in to Drops to choose which badge you wear.
                  </div>
                )}
              </div>
            </CollapsibleSection>
          )
        )}
      </div>
    </DrillInScreen>
  );
};
