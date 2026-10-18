// Your identity and your picks, shared by the You tab's identity card, the 7TV
// account row and Edit profile, so a change made in Edit profile shows on the
// card the moment you go back, with no second read.
//
// Rust builds every answer: `own_identity_header` (saved for the next cold
// launch, so `own_identity_header_cached` paints the first frame), the two
// picker halves (`cosmetics_picker_state`, `external_cosmetics_state`) and the
// Twitch bio. This store only holds them and lets the screens flip a pick at
// once and put it back when the write is refused, as the iPhone's
// CosmeticsScreenModel does.
import { useEffect } from 'react';
import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { subscribeResolvedIdentity } from '../../services/identityService';
import { useAppStore } from '../../stores/AppStore';
import type {
  ExternalPickerView,
  OwnIdentityView,
  StreamNookPickerView,
} from '../../types/profileView';
import { connectSevenTv as runSevenTvSignIn, getSevenTvStatus } from '../cosmetics/sevenTvConnect';
import { Logger } from '../../utils/logger';

export interface PicksUser {
  userId: string;
  login: string;
  displayName: string;
}

interface PicksState {
  /** Whose answers these are; a sign-out or account switch drops them. */
  userId: string | null;
  identity: OwnIdentityView | null;
  sn: StreamNookPickerView | null;
  ext: ExternalPickerView | null;
  /** The account's Twitch bio; null until read. */
  bio: string | null;
  /** Whether a 7TV session is stored, and whose. Read on its own because the
   *  external half waits on 7TV's paint catalog and the account row should not. */
  sevenTv: { signedIn: boolean; userId: string | null } | null;
  /** A 7TV sign-in is on screen. */
  connecting: boolean;

  /** Your identity: the saved copy first when there is none yet, then fresh. */
  loadIdentity: (user: PicksUser) => Promise<void>;
  /** Both picker halves, the bio and the 7TV status; each lands on its own. */
  loadPicks: (user: PicksUser, opts?: { external?: boolean }) => Promise<void>;
  patchSn: (fn: (sn: StreamNookPickerView) => StreamNookPickerView) => void;
  patchExt: (fn: (ext: ExternalPickerView) => ExternalPickerView) => void;
  setSn: (sn: StreamNookPickerView | null) => void;
  setExt: (ext: ExternalPickerView | null) => void;
  setBio: (bio: string) => void;
  /** Runs the 7TV sign-in; answers the 7TV account id once a session is
   *  stored, or null (with a toast saying why when it is not just a cancel). */
  connectSevenTv: (twitchUserId: string) => Promise<string | null>;
  disconnectSevenTv: () => Promise<boolean>;
  /** 7TV refused a write because the session lapsed. */
  sevenTvExpired: () => void;
}

/** One read of each kind at a time: the You card and Edit profile both ask
 *  on a registry change, and the second ask joins the first. */
const inFlight = new Map<string, Promise<void>>();
const once = (key: string, run: () => Promise<void>): Promise<void> => {
  const running = inFlight.get(key);
  if (running) return running;
  const p = run().finally(() => inFlight.delete(key));
  inFlight.set(key, p);
  return p;
};

/** Drops answers that belong to another account. */
const forUser = (get: () => PicksState, userId: string) => get().userId === userId;

export const usePicksStore = create<PicksState>((set, get) => {
  const toast = (message: string) => useAppStore.getState().addToast(message, 'error');

  /** Starts a fresh slate when the account changed. */
  const claim = (userId: string) => {
    if (get().userId === userId) return;
    set({ userId, identity: null, sn: null, ext: null, bio: null, sevenTv: null, connecting: false });
  };

  const setSevenTv = (signedIn: boolean, userId: string | null | undefined) =>
    set((s) => ({
      sevenTv: { signedIn, userId: userId ?? s.sevenTv?.userId ?? null },
      ext: s.ext
        ? { ...s.ext, seventv_signed_in: signedIn, seventv_user_id: userId ?? s.ext.seventv_user_id }
        : s.ext,
    }));

  return {
    userId: null,
    identity: null,
    sn: null,
    ext: null,
    bio: null,
    sevenTv: null,
    connecting: false,

    loadIdentity: ({ userId, login, displayName }) => {
      if (!userId) return Promise.resolve();
      claim(userId);
      return once(`identity:${userId}`, async () => {
        if (!get().identity) {
          const cached = await invoke<OwnIdentityView | null>('own_identity_header_cached', { userId }).catch(
            () => null,
          );
          if (cached && forUser(get, userId) && !get().identity) set({ identity: cached });
        }
        try {
          const fresh = await invoke<OwnIdentityView>('own_identity_header', {
            userId,
            login: login || displayName,
            displayName,
          });
          if (forUser(get, userId)) set({ identity: fresh });
        } catch (err) {
          Logger.debug('[Picks] own identity unavailable:', err);
        }
      });
    },

    loadPicks: ({ userId, login }, opts) => {
      if (!userId) return Promise.resolve();
      claim(userId);
      return once(`picks:${userId}:${opts?.external !== false}`, async () => {
        const reads: Promise<void>[] = [
          invoke<StreamNookPickerView>('cosmetics_picker_state', { userId })
            .then((sn) => {
              if (forUser(get, userId)) set({ sn });
            })
            .catch((err) => Logger.warn('[Picks] picker state unavailable:', err)),
          invoke<string>('get_twitch_bio', { userId })
            .then((bio) => {
              if (forUser(get, userId)) set({ bio });
            })
            .catch((err) => Logger.debug('[Picks] bio unavailable:', err)),
          getSevenTvStatus()
            .then((status) => {
              if (forUser(get, userId)) setSevenTv(status.is_authenticated, status.user_id);
            })
            .catch((err) => Logger.debug('[Picks] 7TV status unavailable:', err)),
        ];
        if (opts?.external !== false) {
          reads.push(
            invoke<ExternalPickerView>('external_cosmetics_state', { userId, login })
              .then((ext) => {
                if (!forUser(get, userId)) return;
                set((s) => ({
                  ext: { ...ext, named_colors: ext.named_colors ?? [] },
                  sevenTv: {
                    signedIn: ext.seventv_signed_in,
                    userId: ext.seventv_user_id ?? s.sevenTv?.userId ?? null,
                  },
                }));
              })
              .catch((err) => Logger.debug('[Picks] 7TV and Twitch cosmetics unavailable:', err)),
          );
        }
        await Promise.all(reads);
      });
    },

    patchSn: (fn) => set((s) => (s.sn ? { sn: fn(s.sn) } : {})),
    patchExt: (fn) => set((s) => (s.ext ? { ext: fn(s.ext) } : {})),
    setSn: (sn) => set({ sn }),
    setExt: (ext) => set({ ext }),
    setBio: (bio) => set({ bio }),

    connectSevenTv: async (twitchUserId) => {
      if (get().connecting) return null;
      set({ connecting: true });
      try {
        if (!(await runSevenTvSignIn(twitchUserId))) return null;
        const status = await getSevenTvStatus();
        setSevenTv(status.is_authenticated, status.user_id);
        if (!status.is_authenticated) {
          // A token was stored but does not count as a session (expired or
          // refused). Saying so is what keeps Connect from looking dead.
          toast('7TV did not finish signing you in. Try again.');
          return null;
        }
        const id = status.user_id ?? get().sevenTv?.userId ?? null;
        if (!id) toast('7TV did not identify your account.');
        return id;
      } catch (err) {
        Logger.error('[Picks] 7TV sign-in failed:', err);
        toast('Could not sign in to 7TV.');
        return null;
      } finally {
        set({ connecting: false });
      }
    },

    disconnectSevenTv: async () => {
      try {
        await invoke('logout_seventv');
        setSevenTv(false, null);
        return true;
      } catch (err) {
        Logger.error('[Picks] 7TV sign-out failed:', err);
        toast('Could not sign out of 7TV.');
        return false;
      }
    },

    sevenTvExpired: () => setSevenTv(false, null),
  };
});

/**
 * Reads your identity and picks while `active`, and re-reads when they change
 * elsewhere: a StreamNook registry change (a badge granted, an Atmosphere
 * added) re-reads the StreamNook half and the identity; a resolved-identity
 * change re-reads the identity. The 7TV half is left alone on those, because
 * 7TV's read lags its own write and a re-read would show the previous pick.
 */
export function useOwnPicks(user: PicksUser | null, active = true): void {
  const userId = user?.userId ?? '';
  const login = user?.login ?? '';
  const displayName = user?.displayName ?? '';
  useEffect(() => {
    if (!active || !userId) return;
    const who: PicksUser = { userId, login, displayName };
    const store = usePicksStore.getState();
    void store.loadIdentity(who);
    void store.loadPicks(who);
    const unsubscribe = subscribeResolvedIdentity((changedId) => {
      if (changedId === userId) void usePicksStore.getState().loadIdentity(who);
    });
    const unlisten = listen('sn-registry-changed', () => {
      const now = usePicksStore.getState();
      void now.loadIdentity(who);
      void now.loadPicks(who, { external: false });
    });
    return () => {
      unsubscribe();
      void unlisten.then((fn) => fn());
    };
  }, [active, userId, login, displayName]);
}
