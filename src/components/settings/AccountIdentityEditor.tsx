import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { listen } from '@tauri-apps/api/event';
import { X, Loader2, Link2, Unlink, Check, User } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { getFullProfileWithFallback, forceRefreshCosmetics, type CachedProfile } from '../../services/cosmeticsCache';
import { computePaintStyle, getBadgeImageUrl } from '../../services/seventvService';
import { getIdentityWithCache, setIdentity } from '../../services/identityService';
import { Tooltip } from '../ui/Tooltip';
import {
  getSeventvStatusForAccount,
  connectSeventvForAccount,
  disconnectSeventvForAccount,
  validateSeventvForAccount,
  refreshSeventvForAccount,
  setSeventvPaintForAccount,
  setSeventvBadgeForAccount,
  type SevenTVAccountStatus,
  type StoredAccount,
} from '../../services/accountService';
import { SevenTvAvatar } from '../SevenTvAvatar';

const tpKey = (b: { provider: string; id: string }) => `${b.provider}:${b.id}`;

/**
 * Edit one linked account's StreamNook identity — its 7TV cosmetics (connected
 * through an isolated login window so the alt's Twitch session never touches the
 * main) and its StreamNook badge loadout. Reached from the Linked-accounts list.
 */
export default function AccountIdentityEditor({
  account,
  onClose,
}: {
  account: StoredAccount;
  onClose: () => void;
}) {
  const addToast = useAppStore((s) => s.addToast);
  const mountedRef = useRef(true);

  const [profile, setProfile] = useState<CachedProfile | null>(null);
  const [status, setStatus] = useState<SevenTVAccountStatus | null>(null);
  const [loadoutBadges, setLoadoutBadges] = useState<string[]>([]);
  const [selectedPaintId, setSelectedPaintId] = useState<string | null>(null);
  const [selectedBadgeId, setSelectedBadgeId] = useState<string | null>(null);
  const [connecting, setConnecting] = useState(false);
  const [busy, setBusy] = useState(false);
  const triedRefreshRef = useRef(false);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const loadProfile = async () => {
    const p = await getFullProfileWithFallback(account.user_id, account.login);
    if (!mountedRef.current) return;
    setProfile(p);
    const paints = p.seventvCosmetics?.paints ?? [];
    const badges = p.seventvCosmetics?.badges ?? [];
    setSelectedPaintId((paints.find((x: any) => x.selected)?.id as string) ?? null);
    setSelectedBadgeId((badges.find((x: any) => x.selected)?.id as string) ?? null);
  };

  const loadStatus = async () => {
    let s = await getSeventvStatusForAccount(account.user_id);
    if (!mountedRef.current) return;
    // The status above is the instant local (JWT-exp) read. If it claims
    // connected, confirm authoritatively with 7TV; a revoked token gets cleared
    // server-side and we flip to disconnected so the UI tells the truth.
    if (s.is_authenticated) {
      const ok = await validateSeventvForAccount(account.user_id);
      if (!mountedRef.current) return;
      if (!ok) s = { is_authenticated: false, user_id: null, twitch_id: null };
    }
    setStatus(s);
    if (s.is_authenticated) {
      void loadProfile();
      return;
    }
    // Not connected. Try ONE silent refresh in the background using the persisted
    // session; if it succeeds the UI flips to connected with no interaction. If
    // the session has truly lapsed it fails quietly and the Connect button stands.
    if (!triedRefreshRef.current) {
      triedRefreshRef.current = true;
      void refreshSeventvForAccount(account.user_id)
        .then((ok) => {
          if (ok && mountedRef.current) void loadStatus();
        })
        .catch(() => {});
    }
  };

  useEffect(() => {
    void loadProfile();
    void loadStatus();
    void getIdentityWithCache(account.user_id).then((lo) => {
      if (mountedRef.current) setLoadoutBadges(lo.badges);
    });
    const refresh = () => {
      if (!mountedRef.current) return;
      setConnecting(false);
      void loadStatus();
      void loadProfile();
    };
    // Per-account (incognito) connect targets this account by id.
    const unAccount = listen<string>('seventv-connected-account', (e) => {
      if (e.payload === account.user_id) {
        refresh();
        addToast('7TV connected for this account', 'success');
      }
    });
    // The primary connects through the shared single-slot flow, which emits this.
    const unPrimary = listen('seventv-connected', () => refresh());
    return () => {
      void unAccount.then((fn) => fn());
      void unPrimary.then((fn) => fn());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [account.user_id]);

  const handleConnect = async () => {
    setConnecting(true);
    try {
      await connectSeventvForAccount(account.user_id);
      // Resolves when the window opens; the event listener finishes the flow.
    } catch (e) {
      setConnecting(false);
      addToast(typeof e === 'string' ? e : 'Could not open 7TV login', 'error');
    }
  };

  const handleDisconnect = async () => {
    try {
      await disconnectSeventvForAccount(account.user_id);
      await loadStatus();
      addToast('7TV disconnected', 'info');
    } catch (e) {
      addToast(typeof e === 'string' ? e : 'Could not disconnect 7TV', 'error');
    }
  };

  const seventvUserId = status?.user_id ?? null;
  const connected = !!status?.is_authenticated;

  const pickPaint = async (paintId: string | null) => {
    if (!seventvUserId || busy) return;
    setBusy(true);
    const next = paintId === selectedPaintId ? null : paintId;
    try {
      await setSeventvPaintForAccount(account.user_id, seventvUserId, next);
      if (mountedRef.current) setSelectedPaintId(next);
      // The 7TV write doesn't touch our local cache, so re-resolve this account's
      // cosmetics: the new paint then shows in chat + the profile card right away
      // (repaints via the cosmetics bridge) instead of waiting for the cache TTL.
      void forceRefreshCosmetics(account.user_id);
    } catch (e) {
      addToast(typeof e === 'string' ? e : 'Could not change paint', 'error');
    } finally {
      if (mountedRef.current) setBusy(false);
    }
  };

  const pickBadge = async (badgeId: string | null) => {
    if (!seventvUserId || busy) return;
    setBusy(true);
    const next = badgeId === selectedBadgeId ? null : badgeId;
    try {
      await setSeventvBadgeForAccount(account.user_id, seventvUserId, next);
      if (mountedRef.current) setSelectedBadgeId(next);
      // Re-resolve so the new badge shows in chat + the profile card immediately
      // (the 7TV write doesn't touch our local cache). Repaints via the bridge.
      void forceRefreshCosmetics(account.user_id);
    } catch (e) {
      addToast(typeof e === 'string' ? e : 'Could not change badge', 'error');
    } finally {
      if (mountedRef.current) setBusy(false);
    }
  };

  const toggleThirdParty = (b: { provider: string; id: string }) => {
    const key = tpKey(b);
    const next = loadoutBadges.includes(key)
      ? loadoutBadges.filter((k) => k !== key)
      : [...loadoutBadges, key];
    setLoadoutBadges(next);
    // Authenticated as THIS account (the server writes its row, not the main's).
    void setIdentity(account.user_id, next, null, true, account.user_id).catch((e) => {
      addToast(typeof e === 'string' ? e : 'Could not save badges', 'error');
    });
  };

  const paints = profile?.seventvCosmetics?.paints ?? [];
  const badges = profile?.seventvCosmetics?.badges ?? [];
  const thirdParty = profile?.thirdPartyBadges ?? [];

  return createPortal(
    <div
      className="fixed inset-0 z-[1000] flex items-center justify-center bg-black/60 p-4"
      onMouseDown={onClose}
    >
      <div
        className="glass-panel rounded-2xl w-full max-w-lg max-h-[85vh] overflow-y-auto scrollbar-thin p-5 space-y-5"
        onMouseDown={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center gap-3">
          {account.avatar_url ? (
            <SevenTvAvatar
              cosmeticsId={account.user_id}
              px={40}
              motion="animate"
              src={account.avatar_url}
              alt=""
              className="w-10 h-10 rounded-full object-cover"
            />
          ) : (
            <div className="w-10 h-10 rounded-full bg-white/5 flex items-center justify-center">
              <User size={18} className="text-textMuted" />
            </div>
          )}
          <div className="min-w-0 flex-1">
            <div className="text-sm font-semibold text-textPrimary truncate">
              {account.display_name || account.login}
            </div>
            <div className="text-xs text-textSecondary truncate">Editing identity · @{account.login}</div>
          </div>
          <button
            onClick={onClose}
            aria-label="Close"
            className="p-1.5 text-textMuted hover:text-textPrimary hover:bg-white/5 rounded-md transition-colors"
          >
            <X size={18} />
          </button>
        </div>

        {/* 7TV */}
        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <h4 className="text-sm font-semibold text-textPrimary uppercase tracking-wide">7TV</h4>
            {connected && (
              <button
                onClick={handleDisconnect}
                className="flex items-center gap-1 text-[11px] text-textMuted hover:text-red-400 transition-colors"
              >
                <Unlink size={12} />
                Disconnect
              </button>
            )}
          </div>

          {!connected ? (
            <div className="rounded-lg bg-white/[0.03] p-3 space-y-2">
              <p className="text-xs text-textSecondary">
                Connect this account's 7TV to manage its paint and badge. A separate sign-in window
                opens. Sign in to Twitch as <span className="text-textPrimary">@{account.login}</span>{' '}
                there; your main is not affected.
              </p>
              <button
                onClick={handleConnect}
                disabled={connecting}
                className="flex items-center gap-1.5 px-3 py-1.5 text-sm font-medium glass-button disabled:cursor-wait disabled:opacity-70"
              >
                {connecting ? <Loader2 size={14} className="animate-spin" /> : <Link2 size={14} />}
                {connecting ? 'Waiting for sign-in…' : 'Connect 7TV'}
              </button>
            </div>
          ) : (
            <>
              {/* Paints */}
              <div>
                <div className="text-[11px] text-textSecondary mb-1.5">Paint</div>
                {paints.length === 0 ? (
                  <p className="text-xs text-textMuted">No paints owned on this account.</p>
                ) : (
                  <div className="flex flex-wrap gap-1.5">
                    {paints.map((paint: any) => {
                      const active = paint.id === selectedPaintId;
                      return (
                        <button
                          key={paint.id}
                          onClick={() => pickPaint(paint.id)}
                          disabled={busy}
                          className={`px-2.5 py-1 rounded-md text-xs font-semibold transition-all disabled:opacity-60 ${
                            active ? 'glass-input ring-1 ring-accent/50' : 'bg-white/[0.04] hover:bg-white/[0.08]'
                          }`}
                          style={computePaintStyle(paint)}
                        >
                          {paint.name || 'Paint'}
                        </button>
                      );
                    })}
                  </div>
                )}
              </div>

              {/* Badges */}
              <div>
                <div className="text-[11px] text-textSecondary mb-1.5">Badge</div>
                {badges.length === 0 ? (
                  <p className="text-xs text-textMuted">No badges owned on this account.</p>
                ) : (
                  <div className="flex flex-wrap gap-2">
                    {badges.map((badge: any) => {
                      const active = badge.id === selectedBadgeId;
                      return (
                        <Tooltip key={badge.id} content={badge.name}>
                        <button
                          onClick={() => pickBadge(badge.id)}
                          disabled={busy}
                          className={`relative p-1.5 rounded-lg transition-all disabled:opacity-60 ${
                            active
                              ? 'glass-input ring-1 ring-accent/40'
                              : 'border border-transparent opacity-60 hover:opacity-90 hover:bg-glass'
                          }`}
                        >
                          <img src={getBadgeImageUrl(badge)} alt={badge.name} className="w-7 h-7" />
                          {active && (
                            <span className="absolute -top-1 -right-1 bg-accent rounded-full p-0.5">
                              <Check size={9} className="text-black" />
                            </span>
                          )}
                        </button>
                        </Tooltip>
                      );
                    })}
                  </div>
                )}
              </div>
            </>
          )}
        </div>

        {/* StreamNook badge loadout (third-party badges) */}
        {thirdParty.length > 0 && (
          <div className="space-y-1.5">
            <h4 className="text-sm font-semibold text-textPrimary uppercase tracking-wide">
              Other badges
            </h4>
            <p className="text-xs text-textSecondary">
              Pick which appear next to this account's name in chat for other StreamNook users.
            </p>
            <div className="flex flex-wrap gap-2 pt-1">
              {thirdParty.map((badge: any, idx: number) => {
                const shown = loadoutBadges.includes(tpKey(badge));
                return (
                  <Tooltip key={`${badge.provider}-${badge.id}-${idx}`} content={`${badge.title} (${String(badge.provider).toUpperCase()})`}>
                  <button
                    onClick={() => toggleThirdParty(badge)}
                    className={`relative p-2 rounded-lg transition-all ${
                      shown
                        ? 'glass-input ring-1 ring-accent/30'
                        : 'border border-transparent opacity-50 hover:opacity-80 hover:bg-glass'
                    }`}
                  >
                    <img src={badge.image4x || badge.imageUrl} alt={badge.title} className="w-8 h-8" />
                    {shown && (
                      <span className="absolute -top-1 -right-1 bg-accent rounded-full p-0.5">
                        <Check size={9} className="text-black" />
                      </span>
                    )}
                  </button>
                  </Tooltip>
                );
              })}
            </div>
          </div>
        )}
      </div>
    </div>,
    document.body,
  );
}
