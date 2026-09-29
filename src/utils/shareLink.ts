// Single source of truth for the shape of a shared stream link.
//
// Share buttons copy the web link (streamnook.app/w/<channel>) rather than the
// raw streamnook:// protocol so the link works for everyone: the landing page
// hands off to the app when it's installed and falls back to Twitch when it
// isn't. The app protocol below is what that page hands off to, and what the
// Rust deep-link handler parses back into a channel.

const SHARE_BASE = 'https://streamnook.app/w';

/** The web link to copy/share for a channel login. */
export function buildShareUrl(channelLogin: string): string {
  // Day tag (MMDD). Discord caches a link's preview per exact URL for ~a day, so a
  // plain /w/<channel> link re-shared later shows the card Discord cached earlier
  // (e.g. last night's stream on a different game). Tagging the day keeps the link
  // clean and readable while making a link shared on a new day a fresh URL that
  // Discord re-crawls. The landing page ignores this for routing and mirrors it
  // onto the card image URL.
  const d = new Date();
  const tag = `${String(d.getMonth() + 1).padStart(2, '0')}${String(d.getDate()).padStart(2, '0')}`;
  return `${SHARE_BASE}/${encodeURIComponent(normalizeLogin(channelLogin))}?d=${tag}`;
}

/** A StreamNook member's public profile page on the web. */
export function buildProfileUrl(login: string): string {
  return `https://streamnook.app/u/${encodeURIComponent(normalizeLogin(login))}`;
}

/** The direct app-protocol link the landing page hands off to. */
export function buildDeepLink(channelLogin: string): string {
  return `streamnook://watch/${encodeURIComponent(normalizeLogin(channelLogin))}`;
}

// Defensive: callers pass a stream's user_login, which is normally present, but
// guard against null/undefined/whitespace so a Share click can never throw.
function normalizeLogin(channelLogin: string | null | undefined): string {
  return (channelLogin ?? '').trim().toLowerCase();
}
