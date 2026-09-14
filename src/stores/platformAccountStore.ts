import { create } from 'zustand';
import * as platformAccounts from '../services/platformAccountService';
import type { PlatformId } from '../services/platformAccountService';
import { useAppStore } from './AppStore';
import { useFollowsStore } from './followsStore';
import { clearLinkedAccount, recordLinkedAccount } from '../services/supabaseService';
import { Logger } from '../utils/logger';

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
  /** A connect/disconnect is in flight. Blocks a second one. */
  busy: boolean;
  /** What the connect flow is doing right now, shown instead of the subtitle. */
  step: string | null;
}

interface PlatformAccountStore {
  kick: PlatformAccountState;
  youtube: PlatformAccountState;
  /** Re-read connection state from the backend for both platforms. */
  refresh: () => Promise<void>;
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
  busy: false,
  step: null,
};

/** How long a terminal step message stays up before the row returns to normal. */
const STEP_CLEAR_MS = 6000;
/** Progress messages, as opposed to outcome messages. Only these get cleared when
 *  the flow finishes — an outcome ("Sign-in was not completed") owns its own
 *  lifetime and must not be wiped the instant it appears. */
const PROGRESS_STEPS = ['Waiting for you to sign in…', 'Getting your channels…'];

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

  /**
   * What we last told the backend about each platform. `null` means "nothing
   * reported yet", which is also what a report that could not be sent falls
   * back to, so the next read retries it.
   */
  const reported: Record<PlatformId, boolean | null> = { kick: null, youtube: null };

  /**
   * Tell the account database that a platform was connected or disconnected.
   *
   * Reports CHANGES, never a heartbeat. `refreshOne` runs on every window focus
   * and on the five-minute session check, and "still connected" is not news; the
   * row's own `last_seen` moves whenever something real happens.
   *
   * Connecting Kick while signed out of Twitch records nothing, because the
   * record is keyed on the Twitch identity. Leaving `reported` unset in that
   * case means the next refresh tries again, so signing into Twitch afterwards
   * backfills it with no extra wiring.
   */
  const report = (provider: PlatformId, connected: boolean, state: PlatformAccountState) => {
    if (reported[provider] === connected) return;
    const twitchUserId = useAppStore.getState().currentUser?.user_id;
    if (!twitchUserId) return;
    reported[provider] = connected;
    void (connected
      ? recordLinkedAccount(twitchUserId, provider, state.name, state.avatarUrl)
      : clearLinkedAccount(twitchUserId, provider));
  };

  const refreshOne = async (provider: PlatformId) => {
    try {
      const connected = await platformAccounts.isConnected(provider);
      if (!connected) {
        const wasConnected = get()[provider].connected;
        patch(provider, { connected, name: null, avatarUrl: null });
        // Deliberately NOT clearProvider here. A session dying mid-run must not
        // blank the sidebar: Kick liveness never needed the login (the sweep
        // runs on an app token) and the follow list survives backend-side for
        // both platforms. Re-reading it keeps the view honest either way — an
        // explicit disconnect purged it backend-side, so hydrate empties the
        // list then; a mere token death leaves it intact and live. The
        // user-facing "your session expired" signal is the dedicated
        // platform-session-expired event, not a vanished sidebar.
        if (wasConnected) void useFollowsStore.getState().hydrate();
        report(provider, false, get()[provider]);
        return;
      }
      const info = await platformAccounts.accountInfo(provider);
      patch(provider, { connected, name: info.name, avatarUrl: info.avatar_url });
      report(provider, true, get()[provider]);
    } catch (e) {
      // Leave the previous state: an IPC failure is not evidence of a sign-out.
      Logger.debug(`[platform] could not read ${provider} state:`, e);
    }
  };

  return {
    kick: { ...IDLE },
    youtube: { ...IDLE },

    refresh: async () => {
      await Promise.all([refreshOne('kick'), refreshOne('youtube')]);
    },

    connect: async (provider) => {
      if (get()[provider].busy) return;
      patch(provider, { busy: true, step: 'Waiting for you to sign in…' });
      try {
        if (provider === 'kick') {
          // ONE window takes consent and then promotes the login into a kick.com
          // session, reading the follow list on the way out. The follow half
          // failing never blocks the connection, so its state is read back below
          // rather than inferred from this call succeeding.
          await useFollowsStore.getState().syncKick(true);
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
        transientStep(
          provider,
          String(e).includes('cancelled') ? 'Sign-in was cancelled' : 'Sign-in was not completed',
        );
      }
      await refreshOne(provider);
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
