// Keeping the 7TV session alive.
//
// 7TV mints 30-day session tokens and the app reads an expired one as signed
// out, so without this every account "keeps getting signed out of 7TV" once a
// month. Each shell has a way to re-mint silently (desktop reloads the login
// in a hidden window, the phone in a hidden overlay); this decides WHEN to
// try, the same way on both, and never more than once per window.
import { invoke } from '@tauri-apps/api/core';
import { Logger } from '../utils/logger';

/** Start trying this far ahead of the expiry. */
export const REFRESH_AHEAD_MS = 7 * 24 * 60 * 60 * 1000;
/** A failed attempt is not retried for this long. */
export const RETRY_WINDOW_MS = 12 * 60 * 60 * 1000;
const STAMP_KEY = 'sn-7tv-refresh-attempt';

export interface SevenTvSessionStatus {
  is_authenticated: boolean;
  user_id: string | null;
  twitch_id: string | null;
  /** JWT exp, unix seconds. Null when no token is stored. */
  expires_at: number | null;
}

/**
 * Whether a silent refresh is worth attempting right now.
 *
 * Pure so it can be tested: a token is worth refreshing once it is inside the
 * lead window or already expired, unless an attempt was made inside the retry
 * window. No stored token at all (never connected, or signed out on purpose)
 * is never refreshed.
 */
export function shouldAttemptSevenTvRefresh(a: {
  expiresAtSec: number | null;
  nowMs: number;
  lastAttemptMs: number | null;
}): boolean {
  if (a.expiresAtSec == null) return false;
  if (a.lastAttemptMs != null && a.nowMs - a.lastAttemptMs < RETRY_WINDOW_MS) return false;
  return a.expiresAtSec * 1000 - a.nowMs <= REFRESH_AHEAD_MS;
}

function readStamp(): number | null {
  try {
    const raw = localStorage.getItem(STAMP_KEY);
    const n = raw ? Number(raw) : NaN;
    return Number.isFinite(n) ? n : null;
  } catch {
    return null;
  }
}

function writeStamp(nowMs: number): void {
  try {
    localStorage.setItem(STAMP_KEY, String(nowMs));
  } catch {
    /* private mode, nothing to remember with */
  }
}

/**
 * Check the stored 7TV token and, if it is close to expiring or already has,
 * run the shell's silent re-mint. `runRefresh` resolves true when a fresh
 * token was captured. Returns what it did, for the log.
 */
export async function maybeRefreshSevenTvSession(
  runRefresh: () => Promise<boolean>,
): Promise<'skipped' | 'refreshed' | 'failed'> {
  let status: SevenTvSessionStatus;
  try {
    status = await invoke<SevenTvSessionStatus>('get_seventv_auth_status');
  } catch (err) {
    Logger.debug('[7TV] session status unavailable:', err);
    return 'skipped';
  }
  const nowMs = Date.now();
  if (!shouldAttemptSevenTvRefresh({ expiresAtSec: status.expires_at, nowMs, lastAttemptMs: readStamp() })) {
    return 'skipped';
  }
  writeStamp(nowMs);
  try {
    const ok = await runRefresh();
    Logger.info(ok ? '[7TV] session re-minted silently' : '[7TV] silent re-mint did not complete');
    return ok ? 'refreshed' : 'failed';
  } catch (err) {
    Logger.warn('[7TV] silent re-mint failed:', err);
    return 'failed';
  }
}
