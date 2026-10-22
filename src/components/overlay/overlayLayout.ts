// Pure layout decisions for the overlay renderer, kept out of OverlayChat.tsx
// so that file exports only components (fast refresh) and these stay testable.
// Synced to the hosted overlay alongside the renderer.

import type { MessageSegment } from '../../services/twitchChat';
import type { OverlayStyle } from './overlayConfig';
import { nameColorReadsLight } from '../../utils/nameColor';

/**
 * Which zero-width emotes sit on top of which emote. A zero-width emote (7TV's
 * overlay flag) draws over the emote or emoji before it, across at most one
 * whitespace-only run, the way chat clients stack them; several in a row all
 * land on the same base. One with nothing to sit on renders as a normal emote.
 * `attached` maps a base index to its overlay indices; `skip` holds the
 * overlays and the spaces they bridged, which render inside the stack instead.
 * `drawsImage` says whether an overlay will draw as a picture: one shown as its
 * typed word (a hidden personal emote) stays in the text flow, never on top.
 */
export function zeroWidthLayout(
  segs: MessageSegment[],
  drawsImage: (s: MessageSegment) => boolean = () => true,
): { attached: Map<number, number[]>; skip: Set<number> } {
  const attached = new Map<number, number[]>();
  const skip = new Set<number>();
  let base = -1;
  let gap = -1;
  segs.forEach((s, i) => {
    if (s.type === 'emote' && s.is_zero_width && base >= 0 && drawsImage(s)) {
      const list = attached.get(base) ?? [];
      list.push(i);
      attached.set(base, list);
      skip.add(i);
      if (gap >= 0) skip.add(gap);
      gap = -1;
      return;
    }
    if (s.type === 'text' && s.content.trim() === '' && base >= 0 && gap < 0) {
      gap = i;
      return;
    }
    const canCarry = s.type === 'emote' || s.type === 'emoji';
    base = canCarry ? i : -1;
    gap = -1;
  });
  return { attached, skip };
}

/**
 * Whether names sit on something dark: the bubble when bubbles are on, the panel
 * when the background is solid, otherwise the stream itself, which the text
 * shadow darkens unless the streamer picked a light shadow.
 */
export function overlayBackdropIsDark(style: OverlayStyle): boolean {
  const backdrop = style.bubble
    ? style.bubbleColor || '#0e0e10'
    : style.background === 'solid'
      ? style.backgroundColor
      : style.textShadow !== false
        ? style.textShadowColor || '#000000'
        : '#000000';
  return nameColorReadsLight(backdrop) !== true;
}

/**
 * Which line of a message a GIF starts on, 0 being the name's line, from the
 * boxes of everything drawn before it in the row and the GIF's own top.
 *
 * The GIF hangs from the top of its line (text-top), so a box whose middle is
 * above that top sits on an earlier line, and everything on the GIF's own line
 * (text, middle-aligned badges and emotes) has its middle below it. Boxes that
 * share a line overlap, so they are counted once: a new line starts where a
 * box's middle falls below every box seen so far on the current one.
 */
export function gifLineIndex(before: ReadonlyArray<{ top: number; bottom: number }>, gifTop: number): number {
  const above = before
    .filter((r) => r.bottom > r.top && (r.top + r.bottom) / 2 < gifTop)
    .slice()
    .sort((a, b) => a.top - b.top);
  let lines = 0;
  let lineBottom = Number.NEGATIVE_INFINITY;
  for (const r of above) {
    if ((r.top + r.bottom) / 2 > lineBottom) {
      lines++;
      lineBottom = r.bottom;
    } else {
      lineBottom = Math.max(lineBottom, r.bottom);
    }
  }
  return lines;
}

/** The highest Max lines the overlay applies, whatever a config says. */
export const MAX_MESSAGE_LINES_CAP = 6;

/**
 * The line cap for a message row under Max lines (0 when Max lines is off).
 * A GIF too wide for what is left of the name's line wraps below it, and a
 * plain cap would then cut the GIF's whole line away. So a GIF's line always
 * counts as one line of the cap and the text clamps around it: the cap reaches
 * at least as far as the line the GIF starts on (`gifLine`, from
 * `gifLineIndex`; 0 for a row without one).
 */
export function messageLineClamp(maxMessageLines: number | undefined, gifLine = 0): number {
  const lines = Math.round(maxMessageLines ?? 0);
  if (!(lines >= 1)) return 0;
  return Math.max(Math.min(MAX_MESSAGE_LINES_CAP, lines), gifLine + 1);
}

/**
 * Badge art at the largest size each badge CDN serves. A streamer who sets a
 * small font and then scales the browser source up (or zooms it in OBS) would
 * otherwise see small art stretched; the files are a few KB and cached, so the
 * renderer always asks for the biggest. Unknown hosts pass through unchanged,
 * and the renderer falls back to the URL as given if the bigger file is missing.
 */
export function hiResBadgeUrl(url: string | undefined): string | undefined {
  if (!url || url.startsWith('data:')) return url;
  // Twitch: /1, /2, /3; 3 is the 72 px art.
  if (/^https:\/\/static-cdn\.jtvnw\.net\/badges\/v1\/[^/]+\/[12]$/.test(url)) return url.replace(/\/[12]$/, '/3');
  // 7TV: /1x to /4x, with or without a format. AVIF swaps to WebP for older OBS.
  if (url.includes('cdn.7tv.app/badge/')) {
    return url.replace(/\/[123]x(\.\w+)?$/, (_m, ext?: string) => `/4x${ext === '.avif' ? '.webp' : (ext ?? '')}`);
  }
  // FFZ: /badge/<id>/<1|2|4>[/rounded], and channel room badges ending /<1|2|4>.
  if (url.includes('cdn.frankerfacez.com/')) {
    return url
      .replace(/(\/badge\/\d+)\/[12](?=\/|$|\?)/, '$1/4')
      .replace(/(\/room-badge\/.+)\/[12]$/, '$1/4');
  }
  // YouTube member art: Google image hosts take a size in the URL (=s16, =w16-h16).
  if (/^https:\/\/[^/]*(ggpht\.com|googleusercontent\.com)\//.test(url)) {
    return url.replace(/=s\d+(?=-|$)/, '=s128').replace(/=w\d+-h\d+(?=-|$)/, '=w128-h128');
  }
  return url;
}

/** Where StreamNook's own badge-image cache answers. Absolute, so the app's
 *  builder preview uses it as well as the hosted overlay. */
export const BADGE_CACHE_ORIGIN = 'https://streamnook.app';

/** The badge hosts the site's cache takes (its src/lib/badgeArtHosts.ts holds
 *  the exact path rules; a site test keeps this list in step with it). */
export const BADGE_CACHE_HOSTS: ReadonlySet<string> = new Set([
  'static-cdn.jtvnw.net',
  'cdn.betterttv.net',
  'cdn.frankerfacez.com',
  'api.ffzap.com',
  'cdn.chatterinohomies.com',
  'itzalex.github.io',
  'fourtf.com',
  'tduva.com',
  'api.dank.chat',
  'api.moltorino.com',
]);

const SEVENTV_BADGE = /^https:\/\/cdn\.7tv\.app\/badge\/([0-9A-Z]{26})\/([1-4]x\.(?:webp|avif))$/;

/**
 * A badge image through StreamNook's cache: fetched from its host once per
 * Cloudflare location and kept there, so a badge shows at once instead of
 * waiting on its host (7TV's badge server stalls for seconds at a time), and
 * OBS keeps the file far longer than the hosts' own headers allow. Badge art is
 * never changed in place (a new badge is a new address), so a cached file is
 * never a stale cosmetic. Null when the cache does not take the address; the
 * renderer then loads it from its host, as it also does when the cache fails.
 */
export function cachedBadgeUrl(url: string | undefined): string | null {
  if (!url) return null;
  const stv = SEVENTV_BADGE.exec(url);
  if (stv) return `${BADGE_CACHE_ORIGIN}/api/v1/7tv/badge/${stv[1]}/${stv[2]}`;
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  if (parsed.protocol !== 'https:' || parsed.search || parsed.hash || !BADGE_CACHE_HOSTS.has(parsed.hostname)) return null;
  return `${BADGE_CACHE_ORIGIN}/api/v1/badge-art?src=${encodeURIComponent(parsed.href)}`;
}

// Each emote CDN's address as the site's emote store takes it: [provider,
// pattern giving the id and the size]. The site's src/lib/emoteArtHosts.ts
// holds the same shapes; a site test keeps the two in step.
const EMOTE_CACHE_SHAPES: ReadonlyArray<[string, RegExp]> = [
  ['7tv', /^https:\/\/cdn\.7tv\.app\/emote\/([0-9A-Z]{26})\/([1-4]x\.(?:webp|avif))$/],
  ['bttv', /^https:\/\/cdn\.betterttv\.net\/emote\/([0-9a-f]{24})\/([1-3]x(?:\.webp)?)$/],
  ['ffz', /^https:\/\/cdn\.frankerfacez\.com\/emote\/(\d{1,9})\/([124])$/],
  ['twitch', /^https:\/\/static-cdn\.jtvnw\.net\/emoticons\/v2\/(\d{1,12}|emotesv2_[0-9a-f]{32})\/((?:static|animated|default)\/dark\/[1-4]\.0)$/],
];

/**
 * An emote image through StreamNook's image store: kept at the edge and in the
 * site's bucket by the emote's id (never its name), so an emote still draws
 * while 7TV or another CDN is slow or down. Null when the store does not take
 * the address; the renderer then loads it from its CDN, as it also does when
 * the store fails.
 */
export function cachedEmoteUrl(url: string | undefined): string | null {
  if (!url) return null;
  const src = url.startsWith('//') ? `https:${url}` : url;
  for (const [provider, shape] of EMOTE_CACHE_SHAPES) {
    const m = shape.exec(src);
    if (m) return `${BADGE_CACHE_ORIGIN}/api/v1/emote-art/${provider}/${m[1]}/${m[2]}`;
  }
  return null;
}

// Prefer a raster (webp/gif) emote URL over AVIF: OBS's embedded Chromium can be
// old and may not decode AVIF, leaving emotes blank. 7TV serves both.
export const preferRasterEmote = (url: string): string =>
  (url || '').includes('cdn.7tv.app') ? url.replace(/\.avif(\b|$)/i, '.webp') : url;

// Reconstruct an emote's URL from its id, the same resolution the in-app chat row
// does (ChatMessage builds `.../emoticons/v2/{id}/...` when the tokenized URL is
// absent). This is why a Twitch/7TV emote never renders broken just because its
// baked URL wasn't accessible: we resolve it, we don't just hide it. 7TV ids are
// 24/26 chars; everything else is a numeric Twitch id.
const is7tvId = (id: string): boolean => id.length === 24 || id.length === 26;
export const emoteUrlFromId = (id: string): string =>
  is7tvId(id)
    ? `https://cdn.7tv.app/emote/${id}/3x.webp`
    : `https://static-cdn.jtvnw.net/emoticons/v2/${id}/default/dark/3.0`;

// 4x-equivalent art for a gigantified emote. 7TV has a real 4x tier; Twitch
// serves size 4.0 (legacy emotes alias 3.0, the CSS height upscales, same as
// FFZ's client); BTTV tops out at 3x, FFZ at /4. Unknown CDNs upscale their
// inline art.
export const giantEmoteUrl = (url: string, id?: string): string => {
  if (id && is7tvId(id)) return `https://cdn.7tv.app/emote/${id}/4x.webp`;
  if (id && /^\d+$/.test(id) && (!url || url.includes('jtvnw.net')))
    return `https://static-cdn.jtvnw.net/emoticons/v2/${id}/default/dark/4.0`;
  if (url.includes('cdn.7tv.app')) return url.replace(/\/\dx(\.\w+)?$/, '/4x$1');
  if (url.includes('jtvnw.net')) return url.replace(/\/[\d.]+$/, '/4.0');
  if (url.includes('frankerfacez')) return url.replace(/\/[124]$/, '/4');
  return url;
};

// The addresses an emote tries, in order: StreamNook's image store (kept by the
// emote's id, so it draws while 7TV or another CDN is down), then the CDN's own
// address, then the one rebuilt from the id (a stale or wrong baked URL still
// resolves), each through the store first. Giant mode (gigantified emotes) asks
// the 4x art first and degrades to the inline art upscaled, with the stored
// inline art ahead of the CDN's 4x: the prefill keeps the inline size, so that
// is what an outage leaves.
export const emoteCandidates = (segment: { emote_url?: string; emote_id?: string }, giant = false): string[] => {
  const rebuilt = segment.emote_id ? emoteUrlFromId(segment.emote_id) : '';
  const inline = preferRasterEmote(segment.emote_url || '') || rebuilt;
  const primary = giant ? preferRasterEmote(giantEmoteUrl(segment.emote_url || '', segment.emote_id)) || inline : inline;
  const order = giant
    ? [cachedEmoteUrl(primary), cachedEmoteUrl(inline), primary, inline, cachedEmoteUrl(rebuilt), rebuilt]
    : [cachedEmoteUrl(inline), inline, cachedEmoteUrl(rebuilt), rebuilt];
  return Array.from(new Set(order.filter((u): u is string => !!u)));
};
