import { useCallback, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useVisibleInterval } from '../utils/useVisibleInterval';
import { Logger } from '../utils/logger';

/**
 * Periodically confirm the connected Kick / YouTube sessions are still accepted,
 * signing out any the platform has revoked.
 *
 * This exists because `kick_is_connected` / `youtube_is_connected` are pure
 * in-memory reads — they answer "do we hold a token", never "does it still work".
 * Polling those every 5s (which several surfaces used to do, once per pane) could
 * not detect a revoked token, so the Accounts row would keep reading "Connected"
 * for a dead session until the next send failed. One real check on a long period
 * replaces all of that and is strictly more informative.
 *
 * Mount ONCE, in the main window only. The backend broadcasts
 * `platform-account-changed` to every window, so popouts learn without running a
 * check of their own.
 */

/** How often to ask the platforms. Revocation is rare and each check is a real
 *  network round trip, so this is deliberately slow. */
const SESSION_CHECK_MS = 5 * 60 * 1000;
/** Floor between checks, whatever triggers one. `useVisibleInterval` fires on
 *  every visibility regain, so without this, flapping the window (minimise /
 *  restore repeatedly) would turn a 5-minute check into a burst. */
const MIN_GAP_MS = 60 * 1000;
/** Delay before the first check. A session can die while the app is closed, so
 *  waiting a full period would leave the UI lying for five minutes; but firing
 *  during startup would compete with the work that actually renders the app. */
const PRIME_DELAY_MS = 15 * 1000;

export function usePlatformSessionCheck(): void {
  const lastRunAt = useRef(0);

  const check = useCallback(() => {
    const now = Date.now();
    if (now - lastRunAt.current < MIN_GAP_MS) return;
    lastRunAt.current = now;
    invoke<string[]>('validate_platform_sessions')
      .then((signedOut) => {
        if (signedOut.length > 0) {
          Logger.info(`[platform] session no longer valid: ${signedOut.join(', ')}`);
        }
      })
      .catch((e) => {
        // An unreachable platform is not evidence of anything; the backend
        // already refuses to sign anyone out on an inconclusive result.
        Logger.debug('[platform] session check failed:', e);
      });
  }, []);

  useEffect(() => {
    const t = setTimeout(check, PRIME_DELAY_MS);
    return () => clearTimeout(t);
  }, [check]);

  useVisibleInterval(check, SESSION_CHECK_MS);
}
