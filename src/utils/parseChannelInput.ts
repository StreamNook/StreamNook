// Turning something a person typed or pasted into (provider, channel).
//
// Two shapes exist on purpose, and conflating them breaks one of the callers:
//
//   parseYouTubeLink       - LINKS ONLY. For inputs where a bare word must stay
//                            a Twitch login, like the MultiChat add box: if a
//                            typed "xqc" resolved as a YouTube handle it would
//                            hijack every Twitch channel added by name.
//   parseYouTubeIdentifier - links, @handles and UC ids. For inputs where the
//                            platform is already known, like the overlay's
//                            source picker or adding a YouTube channel to a
//                            streamer's links: there is nothing to hijack, and
//                            making someone paste a full URL when they know the
//                            handle is busywork.
//
// TikTok comes in the same two shapes (parseTikTokLink, parseTikTokIdentifier)
// for the same reason.
//
// One YouTube link can't be read offline: a legacy /c/NAME or /user/NAME link
// comes back as its path, and services/channelLookup.ts asks YouTube which
// channel it is before anything is stored.

import type { ProviderId } from '../types/providers';

/** What YouTube allows in a handle: letters and digits from any of its
 *  supported scripts, plus `_`, `-`, `.` and the Latin middle dot. */
const HANDLE = /^[\p{L}\p{M}\p{N}_.\xB7-]+$/u;

/** A handle captured from a link, percent-decoded (a copied link encodes every
 *  non-ASCII letter), or null when it isn't one. */
function decodeHandle(raw: string): string | null {
  let handle: string;
  try {
    handle = decodeURIComponent(raw);
  } catch {
    return null;
  }
  return HANDLE.test(handle) ? handle : null;
}

/** A YouTube video id, UC channel id or @handle from a PASTED LINK. Null for
 *  anything that is not a link, so a typed word stays available to Twitch.
 *  Video and UC ids are case-sensitive, so they come back exactly as pasted.
 *  A legacy /c/ or /user/ link comes back as its path (isYouTubeLegacyPath). */
export function parseYouTubeLink(input: string): string | null {
  const s = input.trim();
  let m = s.match(/youtu\.be\/([A-Za-z0-9_-]{11})/);
  if (m) return m[1];
  // A watch page, or the popout chat (live_chat?v=ID) people copy into OBS.
  if (/youtube\.com\/(?:watch|live_chat)/i.test(s)) {
    m = s.match(/[?&]v=([A-Za-z0-9_-]{11})/);
    if (m) return m[1];
  }
  m = s.match(/youtube\.com\/(?:live|shorts)\/([A-Za-z0-9_-]{11})/i);
  if (m) return m[1];
  m = s.match(/youtube\.com\/channel\/(UC[A-Za-z0-9_-]{22})/i);
  if (m) return m[1];
  m = s.match(/youtube\.com\/@((?:[\p{L}\p{M}\p{N}_.\xB7-]|%[0-9A-Fa-f]{2})+)/iu);
  const handle = m && decodeHandle(m[1]);
  if (handle) return `@${handle}`;
  // Legacy /c/NAME and /user/NAME links. Those names predate handles and are
  // not handles (`/user/MrBeast6000` is MrBeast; `@MrBeast6000` does not
  // exist), so the path comes back as-is for YouTube to resolve.
  m = s.match(/youtube\.com\/(c|user)\/((?:[\p{L}\p{M}\p{N}_.\xB7-]|%[0-9A-Fa-f]{2})+)/iu);
  if (m) return `${m[1].toLowerCase()}/${m[2]}`;
  return null;
}

/** Whether a parsed YouTube identifier is a legacy `c/NAME` or `user/NAME` path,
 *  which names no channel until YouTube resolves it (services/channelLookup). */
export function isYouTubeLegacyPath(id: string): boolean {
  return /^(?:c|user)\//.test(id);
}

/** Whether a YouTube identifier is a `UC…` channel id, as opposed to an
 *  @handle, a video id or a legacy path. */
export function isYouTubeChannelId(id: string): boolean {
  return /^UC[A-Za-z0-9_-]{22}$/.test(id);
}

/** As parseYouTubeLink, but also accepts a typed `UC…` id or a handle with or
 *  without `@`. Only for inputs where YouTube is already the chosen platform. */
export function parseYouTubeIdentifier(input: string): string | null {
  const fromLink = parseYouTubeLink(input);
  if (fromLink) return fromLink;
  const bare = input.trim().replace(/^@+/, '');
  if (!bare) return null;
  // A UC id is stored verbatim: YouTube channel ids are case-SENSITIVE, and the
  // value here is handed to the chat connect unchanged.
  if (isYouTubeChannelId(bare)) return bare;
  // Anything else typed is a handle, an 11-character word included: a bare
  // video id looks exactly like a handle, and a video has a link to paste.
  if (HANDLE.test(bare)) return `@${bare}`;
  return null;
}

/** A TikTok handle (without the `@`) from a PASTED profile or LIVE link. Null
 *  for anything that is not a link, for the same reason as parseYouTubeLink. */
export function parseTikTokLink(input: string): string | null {
  const m = input.trim().match(/tiktok\.com\/@([A-Za-z0-9_.]+)/i);
  return m ? m[1] : null;
}

/** As above, but also accepts a typed handle with or without `@`. Only for
 *  inputs where TikTok is already the chosen platform. */
export function parseTikTokIdentifier(input: string): string | null {
  const fromLink = parseTikTokLink(input);
  if (fromLink) return fromLink;
  const bare = input.trim().replace(/^@/, '');
  return /^[A-Za-z0-9_.]+$/.test(bare) ? bare : null;
}

/** A Kick slug from a pasted link or a `kick:`/`kick/` prefix. */
export function parseKickLink(input: string): string | null {
  const s = input.trim();
  const m =
    s.match(/^(?:https?:\/\/)?(?:www\.)?kick\.com\/(@?[a-z0-9_-]+)/i) ||
    s.match(/^kick[:/](@?[a-z0-9_-]+)$/i);
  return m ? m[1].replace(/^@/, '').toLowerCase() : null;
}

/** A Kick slug from whatever was typed or pasted: a pasted link's slug, else
 *  the text itself, lowercased and cut to what a slug can hold ([a-z0-9_-]).
 *  A typed display name like "ice poseidon" becomes the real slug
 *  "iceposeidon", and nothing a slug can't hold reaches the backend (a space
 *  there crashed the resolver: Tauri window labels reject spaces). */
export function kickSlugFromInput(input: string): string {
  return (parseKickLink(input) ?? input.trim()).toLowerCase().replace(/[^a-z0-9_-]/g, '');
}

/** Whether a Kick slug can be spelled two ways. Kick's API answers only the
 *  exact slug, and whether a username's underscores became hyphens in its slug
 *  depends on the account, so a name with either has to be looked up
 *  (resolveKickSlug in services/channelLookup). */
export function kickSlugHasTwoSpellings(slug: string): boolean {
  return /[_-]/.test(slug);
}

/** A Twitch login from a pasted link or a `twitch:` prefix. */
export function parseTwitchLink(input: string): string | null {
  const s = input.trim();
  const m =
    s.match(/^(?:https?:\/\/)?(?:www\.)?twitch\.tv\/([a-z0-9_]+)/i) ||
    s.match(/^twitch[:/]([a-z0-9_]+)$/i);
  return m ? m[1].toLowerCase() : null;
}

/**
 * What platform and channel a pasted link names, when the link itself says so.
 *
 * Link-shaped input only. A bare word is ambiguous across every platform, so it
 * is the caller's job to say which one they meant.
 */
export function parseChannelInput(
  input: string,
): { provider: ProviderId; channel: string } | null {
  const kick = parseKickLink(input);
  if (kick) return { provider: 'kick', channel: kick };
  const twitch = parseTwitchLink(input);
  if (twitch) return { provider: 'twitch', channel: twitch };
  const yt = parseYouTubeLink(input);
  if (yt) return { provider: 'youtube', channel: yt };
  return null;
}

/**
 * The platform a pasted link belongs to, TikTok included, or null for anything
 * that is not a link. For an add box that also has a platform picker: a link
 * overrides the picker, because the link already says where the channel lives.
 */
export function linkPlatformOf(input: string): ProviderId | null {
  const s = input.trim();
  const fromLink = parseChannelInput(s);
  if (fromLink) return fromLink.provider;
  return parseTikTokLink(s) ? 'tiktok' : null;
}

/** A Twitch login: letters, digits and underscores, at most 25 characters. */
export function isTwitchLogin(s: string): boolean {
  return /^[a-z0-9_]{1,25}$/i.test(s);
}

/**
 * What someone typed into the add box, as a channel to link.
 *
 * A pasted link names its own platform. A bare word does not, so it is read as a
 * Kick slug: Kick is the platform people actually share by name, and YouTube
 * channels are addressed by an id or handle that a link or an @ already marks.
 * Anything YouTube-shaped still resolves, because `@name` and `UC…` are
 * unambiguous on their own.
 */
export function parseLinkInput(raw: string): { provider: ProviderId; channel: string } | null {
  const s = raw.trim();
  if (!s) return null;
  const fromLink = parseChannelInput(s);
  if (fromLink) return fromLink;
  if (s.startsWith('@') || isYouTubeChannelId(s)) {
    const yt = parseYouTubeIdentifier(s);
    return yt ? { provider: 'youtube', channel: yt } : null;
  }
  const kick = parseKickLink(`kick:${s}`);
  return kick ? { provider: 'kick', channel: kick } : null;
}
