// Inline link previews for chat.
//
// The Rust `fetch_link_preview` command does the actual network fetch + metadata
// parse (so it sidesteps CORS and the webview never touches arbitrary pasted
// URLs). This module decides WHICH urls are allowed to auto-expand (a trusted-
// domain allowlist) and caches results per webview so repeated links and
// scroll-back re-renders don't refetch.

import { Logger } from '../utils/logger';

export type LinkPreviewKind =
  | 'youtube'
  | 'youtube_channel'
  | 'image'
  | 'generic'
  | 'tweet'
  | 'media'
  | 'clip'
  | 'vod'
  | 'discord'
  | 'steam'
  | 'spotify'
  | 'instagram';

// Mirrors the Rust `LinkPreview` struct (serde keeps snake_case field names).
export interface LinkPreview {
  url: string;
  kind: LinkPreviewKind;
  title: string | null;
  description: string | null;
  image: string | null;
  site_name: string | null;
  // Generic byline. Also the price label ("steam") and the discord.gg/<code>
  // handle ("discord").
  author: string | null;
  author_avatar: string | null;
  video_id: string | null;
  // "clip" kind only.
  view_count: number | null;
  duration: number | null;
  // "discord" kind only: live online + total member counts.
  online_count: number | null;
  member_count: number | null;
}

// Registrable domains we auto-expand. A host matches if it equals one of these
// or is a subdomain of it (so www.youtube.com, m.twitch.tv, i.imgur.com all
// resolve). Everything else stays a plain clickable link — no passive fetch,
// which keeps IP-grabber / shock-image links from auto-loading.
const TRUSTED_BASE_DOMAINS: readonly string[] = [
  'youtube.com',
  'youtu.be',
  'twitch.tv',
  'twitter.com',
  'x.com',
  'imgur.com',
  'giphy.com',
  'gph.is',
  'tenor.com',
  'reddit.com',
  'redd.it',
  'github.com',
  'streamable.com',
  'kick.com',
  'spotify.com',
  'soundcloud.com',
  'tiktok.com',
  // Hand-styled cards (Discord join, Steam store, etc.).
  'discord.gg',
  'discord.com',
  'discordapp.com',
  'steampowered.com',
  'steamcommunity.com',
  'instagram.com',
  // Image hosts StreamNook can upload to (utils/imageUploadHosts): their
  // links must render as image cards, for the sender and everyone else.
  'nuuls.com',
  'catbox.moe',
  'uguu.se',
  // Widened to auto-expand as a generic card (OG-friendly, well-known).
  'bsky.app',
  'vimeo.com',
  'bandcamp.com',
];

// A host matches a base if it equals it or is a subdomain of it (so
// www.youtube.com, m.twitch.tv, open.spotify.com all resolve). The optional
// `userTrusted` list (registrable hosts the user opted into) is matched the
// same way, so trusting `example.com` also covers `www.example.com`.
function hostMatches(host: string, bases: readonly string[]): boolean {
  const h = host.toLowerCase();
  return bases.some((base) => {
    const b = base.toLowerCase();
    return h === b || h.endsWith(`.${b}`);
  });
}

export function isTrustedHost(host: string, userTrusted: readonly string[] = []): boolean {
  return hostMatches(host, TRUSTED_BASE_DOMAINS) || hostMatches(host, userTrusted);
}

// The host we persist when the user opts to trust a source: lowercased and
// stripped of a leading `www.` so the stored entry matches every subdomain
// (and so the settings list reads cleanly). Returns null for an unparseable URL.
export function trustableHost(rawUrl: string): string | null {
  try {
    const u = new URL(rawUrl.startsWith('http') ? rawUrl : `https://${rawUrl}`);
    return u.hostname.replace(/^www\./, '').toLowerCase() || null;
  } catch {
    return null;
  }
}

// True when a host is covered by the built-in allowlist (so the settings UI can
// label a user entry as redundant, and "trust" affordances can hide for hosts
// that are already trusted out of the box).
export function isBuiltInTrusted(host: string): boolean {
  return hostMatches(host, TRUSTED_BASE_DOMAINS);
}

export interface PreviewUrl {
  url: string;
  // Trusted (allowlisted) hosts auto-expand; untrusted ones are still surfaced
  // but the UI offers click-to-load so an arbitrary pasted link is never fetched
  // passively (which would reveal the user's IP to that host).
  trusted: boolean;
}

// Extract previewable URLs from a message body, each tagged trusted/untrusted.
// Trusted and untrusted are capped independently (default 2 each) so neither a
// wall of links nor a run of untrusted links can crowd out the other or spawn a
// wall of cards. Dedupes and strips trailing sentence punctuation that the
// greedy match would otherwise swallow into the URL. Preserves message order.
export function extractPreviewUrls(
  text: string,
  max = 2,
  userTrusted: readonly string[] = [],
): PreviewUrl[] {
  if (!text) return [];
  // Fast path: almost every chat line has no link. Skip the regex + URL parsing
  // (and the array work) entirely unless the text could plausibly contain one.
  if (!text.includes('http') && !text.includes('www.')) return [];
  const re = /(?:https?:\/\/|www\.)[^\s]+/gi;
  const out: PreviewUrl[] = [];
  const seen = new Set<string>();
  let trustedCount = 0;
  let untrustedCount = 0;
  let match: RegExpExecArray | null;
  while ((match = re.exec(text)) !== null) {
    if (trustedCount >= max && untrustedCount >= max) break;
    const raw = match[0].replace(/[.,!?:;'")\]}>]+$/, '');
    const url = raw.startsWith('http') ? raw : `https://${raw}`;
    let host: string;
    try {
      host = new URL(url).hostname;
    } catch {
      continue;
    }
    if (seen.has(url)) continue;
    const trusted = isTrustedHost(host, userTrusted);
    if (trusted ? trustedCount >= max : untrustedCount >= max) continue;
    seen.add(url);
    if (trusted) trustedCount++;
    else untrustedCount++;
    out.push({ url, trusted });
  }
  return out;
}

/**
 * Compact, single-line display label for a URL: drop the scheme and `www.`,
 * keep the host fully visible (so it's never misleading about where the link
 * goes), and truncate a long path/query/hash with an ellipsis.
 */
export function prettyUrlLabel(raw: string, maxLen = 40): string {
  try {
    const u = new URL(raw.startsWith('http') ? raw : `https://${raw}`);
    const host = u.hostname.replace(/^www\./, '');
    const rest = (u.pathname === '/' ? '' : u.pathname) + u.search + u.hash;
    const full = host + rest;
    if (rest && full.length > maxLen) {
      const room = Math.max(0, maxLen - host.length - 1);
      return `${host}${rest.slice(0, room)}…`;
    }
    return full.replace(/\/$/, '') || host;
  } catch {
    return raw.length > maxLen ? `${raw.slice(0, maxLen - 1)}…` : raw;
  }
}

// --- In-flight dedupe (no cache) -------------------------------------------
//
// Chat links are ephemeral, so nothing is stored. The only coordination kept is
// an in-flight map that coalesces truly-concurrent requests for the same URL
// (e.g. the same link visible in two rows at once) into a single fetch. It self-
// empties the instant a request settles — it holds no resolved data.
const inflight = new Map<string, Promise<LinkPreview | null>>();

/**
 * Resolve a preview, coalescing concurrent requests for the same URL. Returns
 * null when there's no usable preview (the UI then just leaves the link as-is).
 * Never throws, never caches.
 */
export function fetchLinkPreview(url: string): Promise<LinkPreview | null> {
  const existing = inflight.get(url);
  if (existing) return existing;

  const promise = (async (): Promise<LinkPreview | null> => {
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      return await invoke<LinkPreview>('fetch_link_preview', { url });
    } catch (err) {
      Logger.debug('[LinkPreview] no preview for', url, err);
      return null;
    } finally {
      inflight.delete(url);
    }
  })();

  inflight.set(url, promise);
  return promise;
}
