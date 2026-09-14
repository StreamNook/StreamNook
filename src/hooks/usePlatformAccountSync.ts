import { useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import { usePlatformAccountStore } from '../stores/platformAccountStore';
import { useFollowsStore } from '../stores/followsStore';
import { useAppStore } from '../stores/AppStore';
import { providerLabel, type ProviderId } from '../types/providers';
import { useVisibleInterval } from '../utils/useVisibleInterval';
import { Logger } from '../utils/logger';

/**
 * Keeps `platformAccountStore` current without polling.
 *
 * Mount ONCE per window. Every surface that needs to know whether Kick or
 * YouTube is connected reads the store; none of them checks for itself. That is
 * the whole point — the previous arrangement had one 5-second poll per pane, so
 * a popout with six panes made six IPC calls every five seconds to read two
 * booleans that had not changed since the user last clicked something.
 *
 * Three triggers:
 *  - `platform-account-changed`, emitted by Rust whenever a token is stored or
 *    cleared, including in another window.
 *  - the window becoming visible, which catches anything that happened while it
 *    was hidden.
 *  - a slow backstop poll, below.
 *
 * The backstop matters. The original per-pane 5-second polls were wasteful, but
 * they were also SELF-HEALING: nothing could get permanently stuck, because the
 * next tick fixed it. An event-only design has single points of failure — a
 * listener that attaches after the emit, an emit path someone forgets to add to
 * a new code path, a backend that restarted. Any of those would leave the
 * composer disabled with no way back. So the events do the fast work and a slow
 * poll guarantees convergence.
 */

/** Backstop cadence. Slow enough to be nearly free (one call per window, gated on
 *  visibility), frequent enough that nothing stays wrong for long. */
const BACKSTOP_MS = 30_000;

/** Guards against a second mount silently doubling the listeners. */
let mounted = 0;

export function usePlatformAccountSync(): void {
  useEffect(() => {
    mounted += 1;
    if (mounted > 1) {
      Logger.warn(
        `[platform] usePlatformAccountSync mounted ${mounted} times in one window; it should be exactly one`,
      );
    }

    const refresh = () => void usePlatformAccountStore.getState().refresh();

    // Paint the real state as soon as the window exists.
    refresh();

    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen('platform-account-changed', refresh).then((fn) => {
      // The listener can resolve after unmount; drop it immediately if so.
      if (disposed) fn();
      else unlisten = fn;
    });

    // A session that died on its own (expired or revoked), as opposed to the
    // user signing out. Say so out loud: the silent version of this is the app
    // looking connected while nothing works, which reads as "nobody is live".
    let unlistenExpired: (() => void) | undefined;
    void listen<string>('platform-session-expired', (e) => {
      const provider = e.payload as ProviderId;
      Logger.info(`[platform] ${provider} session expired`);
      // Kick liveness keeps flowing on the app token, so its rows stay. YouTube
      // liveness needs the session, so its last-known rows are now unknowable;
      // keeping them would be exactly the false reading being fixed here.
      if (provider === 'youtube') {
        useFollowsStore.getState().clearProviderLive(provider);
      }
      useAppStore
        .getState()
        .addToast(
          `Your ${providerLabel(provider)} session expired. Reconnect it in Settings, Accounts.`,
          'warning',
        );
      refresh();
    }).then((fn) => {
      if (disposed) fn();
      else unlistenExpired = fn;
    });

    const onVisibility = () => {
      if (document.visibilityState === 'visible') refresh();
    };
    document.addEventListener('visibilitychange', onVisibility);

    return () => {
      disposed = true;
      mounted -= 1;
      unlisten?.();
      unlistenExpired?.();
      document.removeEventListener('visibilitychange', onVisibility);
    };
  }, []);

  // Backstop: converges on the truth even if every event above was missed.
  useVisibleInterval(() => {
    void usePlatformAccountStore.getState().refresh();
  }, BACKSTOP_MS);
}
