// Connecting a 7TV account from the phone.
//
// Desktop opens a second window for this and cannot read its storage, so it
// injects a script that stuffs the token into an `about:blank#...` fragment and
// polls the window's URL to read it back (`commands/seventv_cosmetics.rs`). The
// Android login overlay is a WebView the app owns, so it reads the value out of
// the page directly and hands it over on the `sn:login-storage` channel.
//
// Everything after the capture is the ungated core: `store_seventv_token` takes
// the token as-is, and equipping goes through the same `set_seventv_paint` /
// `set_seventv_badge` commands the desktop editor calls.
import { invoke } from '@tauri-apps/api/core';
import { Logger } from '../../utils/logger';

/** The key 7TV's own frontend writes its session token under. */
const TOKEN_KEY = '7tv-token';

/** Matches the overlay's watch window, so neither side outlives the other. */
const CAPTURE_TIMEOUT_MS = 5 * 60 * 1000;

export interface SevenTvAuthStatus {
  is_authenticated: boolean;
  user_id: string | null;
  twitch_id: string | null;
}

/**
 * The 7TV user id, read out of the token itself.
 *
 * It is a JWT and its subject claim is the id, which is where desktop gets it
 * too. Nothing else in the flow hands it back, so a token that will not decode
 * is unusable even though it is otherwise valid.
 */
function sevenTvUserIdFromToken(token: string): string {
  const parts = token.split('.');
  if (parts.length !== 3) return '';
  try {
    const b64 = parts[1].replace(/-/g, '+').replace(/_/g, '/');
    const payload = JSON.parse(atob(b64.padEnd(Math.ceil(b64.length / 4) * 4, '=')));
    return String(payload.sub ?? payload.user_id ?? '');
  } catch {
    return '';
  }
}

/** Resolves with the token, or null if the sheet was closed or nothing arrived. */
function awaitToken(): Promise<string | null> {
  return new Promise((resolve) => {
    let settled = false;
    const finish = (token: string | null) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      window.removeEventListener('sn:login-storage', onStorage as EventListener);
      window.removeEventListener('sn:login-cancelled', onCancel);
      resolve(token);
    };
    const onStorage = (e: Event) => {
      const detail = (e as CustomEvent<{ key?: string; value?: string }>).detail;
      if (detail?.key !== TOKEN_KEY || !detail.value) return;
      finish(detail.value);
    };
    const onCancel = () => finish(null);
    const timer = setTimeout(() => finish(null), CAPTURE_TIMEOUT_MS);

    window.addEventListener('sn:login-storage', onStorage as EventListener);
    window.addEventListener('sn:login-cancelled', onCancel);
  });
}

/**
 * Runs the 7TV sign-in and stores the resulting token.
 *
 * `twitchUserId` is recorded alongside it. Desktop leaves that blank; the phone
 * knows who is signed in, and the primary token always lands in the same file
 * either way, so it costs nothing to keep.
 */
export async function connectSevenTv(twitchUserId: string): Promise<boolean> {
  const url = await invoke<string>('get_seventv_login_url');
  // Listening before opening, or a sign-in that is already authorised upstream
  // can complete before there is anything to hear it.
  const pending = awaitToken();
  await invoke('open_mobile_login', {
    url,
    watchStorageKey: TOKEN_KEY,
    title: 'Sign in to 7TV',
  });

  const token = await pending;
  if (!token) return false;

  const userId = sevenTvUserIdFromToken(token);
  if (!userId) {
    Logger.warn('[7TV] token captured but carried no user id');
    return false;
  }

  await invoke('store_seventv_token', {
    accessToken: token,
    userId,
    twitchId: twitchUserId,
  });
  return true;
}

export async function getSevenTvStatus(): Promise<SevenTvAuthStatus> {
  return await invoke<SevenTvAuthStatus>('get_seventv_auth_status');
}
