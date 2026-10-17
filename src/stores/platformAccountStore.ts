import { create } from 'zustand';
import * as platformAccounts from '../services/platformAccountService';
import type { PlatformId } from '../services/platformAccountService';
import { useAppStore } from './AppStore';
import { useFollowsStore } from './followsStore';
import { Logger } from '../utils/logger';
import { useAccountRosterStore, type AccountRoster, type RosterAccount } from './accountRosterStore';

/**
 * Connection state for the platforms that have an account you can connect.
 *
 * This replaces six separate `setInterval(check, 5000)` effects — two in
 * ChatWidget (once per Kick/YouTube pane), two in BlendedChatPane, two in
 * ConnectionsSettings. Every one of them polled `kick_is_connected` /
 * `youtube_is_connected`, which are pure in-memory reads: they answer "do we hold
 * a token", never "does it still work". So they burned an IPC round trip each,
 * per pane, forever, and could not detect the one thing worth detecting.
 *
 * There are NO intervals here. Connection state changes for exactly four reasons,
 * and all four are observable:
 *
 *   1. the user connects or disconnects  -> this store's own actions
 *   2. Rust stored or cleared a token    -> `platform-account-changed` event
 *   3. the session was revoked remotely  -> `usePlatformSessionCheck` (5 min)
 *   4. the window regained focus         -> `usePlatformAccountSync`
 *
 * Connecting is ONE action per platform however many credentials it takes. Kick
 * needs an OAuth token (chat + moderation) AND a kick.com site session (their
 * official API exposes no follow list); YouTube needs a cookie session and a
 * subscriptions import. The user should never learn that, so neither half is a
 * separate button and no user-facing string says "import" or "sync".
 */

export interface PlatformAccountState {
  connected: boolean;
  name: string | null;
  /** The signed-in account's own picture, so Accounts can show WHO is connected
   *  and not merely THAT something is. */
  avatarUrl: string | null;
  /** The id this platform's chat stamps on the account's messages. Null until it
   *  resolves, and permanently null for a YouTube account with no channel. */
  id: string | null;
  /** The @handle, where a platform has one apart from the display name (TikTok,
   *  whose nicknames can be stylized past recognition). */
  handle: string | null;
  /** A connect/disconnect is in flight. Blocks a second one. */
  busy: boolean;
  /** What the connect flow is doing right now, shown instead of the subtitle. */
  step: string | null;
}

interface PlatformAccountStore {
  kick: PlatformAccountState;
  youtube: PlatformAccountState;
  /** Signed in only to play age-restricted LIVEs; TikTok chat stays read-only
   *  whether or not this is connected. */
  tiktok: PlatformAccountState;
  /** Re-read connection state from the backend for every platform. */
  refresh: () => Promise<void>;
  /** Apply a roster Rust pushed on `account-roster-changed`. */
  applyRoster: (roster: AccountRoster) => void;
  /** The one connect action for a platform. */
  connect: (provider: PlatformId) => Promise<void>;
  disconnect: (provider: PlatformId) => Promise<void>;
  /** Re-read YouTube subscriptions without reconnecting. */
  resyncYoutube: () => Promise<void>;
}

const IDLE: PlatformAccountState = {
  connected: false,
  name: null,
  avatarUrl: null,
  id: null,
  handle: null,
  busy: false,
  step: null,
};

/** How long a terminal step message stays up before the row returns to normal. */
const STEP_CLEAR_MS = 6000;
/** Progress messages, as opposed to outcome messages. Only these get cleared when
 *  the flow finishes — an outcome ("Sign-in was not completed") owns its own
 *  lifetime and must not be wiped the instant it appears. */
const PROGRESS_STEPS = ['Waiting for you to sign in…', 'Getting your channels…'];

/**
 * What to tell the user when a connect did not finish.
 *
 * Prefers the backend's OWN sentence. It is the only thing that knows which step
 * failed, and the flat "Sign-in was not completed" this replaced described every
 * outcome identically, including the one where the sign-in window never opened,
 * which is indistinguishable from a dead button. Anything that does not read as
 * a sentence for a person falls back to the generic line rather than putting an
 * error code on screen.
 */
const signInReason = (e: unknown): string => {
  const raw = String((e as { message?: string } | null)?.message ?? e).trim();
  if (!raw || raw === 'undefined' || raw.length > 160) return 'Sign-in was not completed';
  if (/cancel/i.test(raw)) return 'Sign-in was cancelled';
  return raw.charAt(0).toUpperCase() + raw.slice(1);
};

export const usePlatformAccountStore = create<PlatformAccountStore>((set, get) => {
  const patch = (provider: PlatformId, next: Partial<PlatformAccountState>) =>
    set((s) => ({ [provider]: { ...s[provider], ...next } }) as Partial<PlatformAccountStore>);

  /** Show a message, then clear it — but only if nothing else has changed it. */
  const transientStep = (provider: PlatformId, message: string) => {
    patch(provider, { step: message });
    setTimeout(() => {
      if (get()[provider].step === message) patch(provider, { step: null });
    }, STEP_CLEAR_MS);
  };

  // Telling StreamNook's account database which Kick and YouTube accounts are
  // connected (so member cosmetics follow into those chats) is Rust's, on every
  // roster it builds (services/linked_accounts.rs).

  /** Take one platform's row from the roster Rust built. */
  const applyRow = (provider: PlatformId, row: RosterAccount | undefined) => {
    if (!row) return;
    const connected = row.status === 'connected';
    if (!connected) {
      const wasConnected = get()[provider].connected;
      patch(provider, { connected, name: null, avatarUrl: null, id: null, handle: null });
      // Deliberately NOT clearProvider here. A session dying mid-run must not
      // blank the sidebar: Kick liveness never needed the login (the sweep
      // runs on an app token) and the follow list survives backend-side for
      // both platforms. Re-reading it keeps the view honest either way — an
      // explicit disconnect purged it backend-side, so hydrate empties the
      // list then; a mere token death leaves it intact and live. The
      // user-facing "your session expired" signal is the dedicated
      // platform-session-expired event, not a vanished sidebar.
      if (wasConnected) void useFollowsStore.getState().hydrate();
      return;
    }
    patch(provider, {
      connected,
      name: row.name,
      avatarUrl: row.avatar_url,
      id: row.account_id,
      handle: row.handle,
    });
  };

  /** The roster carries every platform at once, so one read serves all three
   *  rows and the title bar alike. */
  const applyRoster = (roster: AccountRoster) => {
    useAccountRosterStore.getState().setRoster(roster);
    for (const provider of ['kick', 'youtube', 'tiktok'] as const) {
      applyRow(provider, roster.main.find((a) => a.id === provider));
    }
  };

  const refreshAll = async () => {
    try {
      applyRoster(await platformAccounts.accountRoster());
    } catch (e) {
      // Leave the previous state: an IPC failure is not evidence of a sign-out.
      Logger.debug('[platform] could not read the account roster:', e);
    }
  };

  /** Kept by name for the connect/disconnect flows; the roster answers for
   *  every platform in one read, so there is nothing per-provider left to ask. */
  const refreshOne = (_provider: PlatformId) => refreshAll();

  return {
    kick: { ...IDLE },
    youtube: { ...IDLE },
    tiktok: { ...IDLE },

    refresh: refreshAll,
    applyRoster,

    connect: async (provider) => {
      if (get()[provider].busy) return;
      patch(provider, { busy: true, step: 'Waiting for you to sign in…' });
      // Set when the flow already said why it did not finish, so the check
      // after the roster read never says it a second time.
      let reported = false;
      try {
        if (provider === 'kick') {
          // ONE window takes consent and then promotes the login into a kick.com
          // session, reading the follow list on the way out. The follow half
          // failing never blocks the connection, so its state is read back below
          // rather than inferred from this call succeeding.
          await useFollowsStore.getState().syncKick(true);
        } else if (provider === 'tiktok') {
          // The sign-in IS the whole connection: there is no follow list to read.
          await platformAccounts.beginTiktokSession();
        } else {
          // Two calls, one action: the sign-in only makes the channels READABLE,
          // so without the second the Following tab stays empty and connecting
          // looks like it did nothing.
          await platformAccounts.beginYoutubeSession();
          patch(provider, { step: 'Getting your channels…' });
          try {
            await useFollowsStore.getState().syncYouTube();
          } catch (e) {
            // Reading the channels failing does not un-sign-you-in, so it is
            // reported on its own rather than failing the whole connection.
            Logger.warn('[platform] youtube channels could not be read:', e);
            transientStep(provider, 'Signed in, but no channels came back');
          }
        }
        // Clear only if we're still showing progress; an outcome message set by
        // the inner catch above must survive to be read.
        const step = get()[provider].step;
        if (step && PROGRESS_STEPS.includes(step)) patch(provider, { step: null });
      } catch (e) {
        Logger.warn(`[platform] ${provider} sign-in failed:`, e);
        const reason = signInReason(e);
        transientStep(provider, reason);
        // Not every surface shows `step`. The in-composer pill is a bare button
        // and the Home empty state shows nothing at all, so a failure there read
        // as a click that did nothing. The toast reaches all of them.
        useAppStore.getState().addToast(reason, 'error');
        reported = true;
      }
      await refreshOne(provider);
      // Kick's sign-in is two steps in one window: the kick.com login, then
      // Kick's Authorize page. Closing it after the login reads the follows and
      // returns without error, yet holds no token, so the row would fall back
      // to "Not signed in" with nothing said about why.
      if (provider === 'kick' && !reported && !get().kick.connected) {
        const reason = 'Sign-in was not completed';
        transientStep(provider, reason);
        useAppStore.getState().addToast(reason, 'error');
      }
      // The follow list is only visible once connected, so paint it now rather
      // than leaving the Following tab empty until something else hydrates.
      await useFollowsStore.getState().hydrate();
      patch(provider, { busy: false });
    },

    disconnect: async (provider) => {
      if (get()[provider].busy) return;
      patch(provider, { busy: true });
      try {
        await platformAccounts.disconnect(provider);
        // An explicit sign-out is the one case where the platform's follows and
        // live rows should vanish immediately (the backend purges its imported
        // follows too). Session deaths keep their rows; see refreshOne.
        useFollowsStore.getState().clearProvider(provider);
      } catch (e) {
        Logger.warn(`[platform] ${provider} disconnect failed:`, e);
      }
      // The backend clears its own imported follows and emits
      // `platform-account-changed`; re-read rather than assuming.
      await Promise.all([refreshOne(provider), useFollowsStore.getState().hydrate()]);
      patch(provider, { busy: false, step: null });
    },

    resyncYoutube: async () => {
      if (get().youtube.busy) return;
      patch('youtube', { busy: true, step: 'Checking your channels…' });
      try {
        const { imported } = await useFollowsStore.getState().syncYouTube();
        transientStep('youtube', `${imported} channel${imported === 1 ? '' : 's'}`);
      } catch (e) {
        Logger.warn('[platform] youtube resync failed:', e);
        transientStep('youtube', 'Could not read your channels');
      }
      patch('youtube', { busy: false });
    },
  };
});
