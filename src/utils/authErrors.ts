// How the Twitch sign-in check reads a failed profile read, the iPhone's way
// (ios SignIn.swift): the core holding a token is what "signed in" means, the
// profile is tried three times a second apart, and only a failure that is not
// the network ends the session. A cold start whose first read hits a network
// hiccup (often while Rust refreshes an expired token) keeps the session and
// tries again, rather than showing the sign-in screen over a live sign-in.

/** The words that mark a failure as the network's, as the iPhone reads them
 *  (`SignIn.isNetwork`). Rust's refresh error says "network" when Twitch was
 *  not reached (`refresh_failure_message` in twitch_service.rs). */
export const NETWORK_AUTH_WORDS = [
  'error sending request',
  'timeout',
  'network',
  'dns error',
  'proxy',
  'failed to fetch',
  '500',
  '502',
  '503',
  '504',
] as const;

export function isNetworkAuthError(message: string): boolean {
  const text = message.toLowerCase();
  return NETWORK_AUTH_WORDS.some((word) => text.includes(word));
}

export const PROFILE_TRIES = 3;
export const PROFILE_RETRY_GAP_MS = 1000;
/** While a held session's profile is out of reach, it is read again this often. */
export const HELD_SESSION_RECHECK_MS = 15_000;

/** The profile, tried `PROFILE_TRIES` times a gap apart: right after a launch
 *  or a sign-in Twitch can take a moment to answer. Throws the last error. */
export async function readProfile<T>(
  read: () => Promise<T>,
  wait: (ms: number) => Promise<void> = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
): Promise<T> {
  let last: unknown;
  for (let attempt = 1; attempt <= PROFILE_TRIES; attempt++) {
    try {
      return await read();
    } catch (e) {
      last = e;
      if (attempt < PROFILE_TRIES) await wait(PROFILE_RETRY_GAP_MS);
    }
  }
  throw last;
}

/** What a failed check means: the core still holds a session the network kept
 *  us from confirming, or the viewer is signed out. */
export function authFailureOutcome(failure: {
  hasCredentials: boolean;
  message: string;
  /** Rust signed the viewer out itself (a new scope): never kept. */
  missingScopes: boolean;
}): 'keep-session' | 'signed-out' {
  if (!failure.hasCredentials || failure.missingScopes) return 'signed-out';
  return isNetworkAuthError(failure.message) ? 'keep-session' : 'signed-out';
}
