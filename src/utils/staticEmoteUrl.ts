// Static (first-frame) variant of an animated emote URL, per provider CDN.
// Used by the "Animate emotes" setting (never, or only while the row is
// hovered) and by the phone's Battery Saver. Pure URL rewriting; the CDNs
// publish the static files themselves.
//
//   7TV     .../emote/<id>/1x.avif        -> .../emote/<id>/1x_static.avif
//   BTTV    .../emote/<id>/1x             -> .../emote/<id>/static/1x.webp
//   FFZ     .../emote/<id>/animated/1     -> .../emote/<id>/1
//   Twitch  .../v2/<id>/default/dark/2.0  -> .../v2/<id>/static/dark/2.0
//
// 7TV and BTTV publish a first frame ONLY for emotes that animate: asking one
// for a still emote (KEKW, EZ, FeelsBadMan) answers 404. Callers skip the
// rewrite for an emote known to be still and fall back to the normal file when
// the first frame is missing (see noStillFrame below).
//
// Anything else (disk copies, unknown hosts) is returned as is.

import { isDiskCopyUrl } from './cacheFileUrl';

export function staticEmoteUrl(url: string): string {
  if (!url || isDiskCopyUrl(url)) return url;
  if (url.includes('cdn.7tv.app/emote/')) {
    if (url.includes('_static.')) return url;
    return url.replace(/\/(\dx)\.(avif|webp|png|gif)(\?.*)?$/, '/$1_static.$2$3');
  }
  if (url.includes('cdn.betterttv.net/emote/')) {
    if (url.includes('/static/')) return url;
    // BTTV serves its first frames as WebP only; `static/1x` without the
    // extension is a 404 even for an animated emote.
    return url.replace(/\/emote\/([^/]+)\/(\dx)(?:\.(?:webp|gif|png))?$/, '/emote/$1/static/$2.webp');
  }
  if (url.includes('cdn.frankerfacez.com/emote/')) {
    return url.replace(/\/emote\/([^/]+)\/animated\/(\d)/, '/emote/$1/$2');
  }
  if (url.includes('static-cdn.jtvnw.net/emoticons/')) {
    return url.replace('/default/', '/static/');
  }
  return url;
}

/** 7TV density srcSet in one format, animated or the static first frame. */
export function sevenTvSrcSet(emoteId: string, ext: 'avif' | 'webp', still = false): string {
  const suffix = still ? '_static' : '';
  return [1, 2, 3, 4]
    .map((n) => `https://cdn.7tv.app/emote/${emoteId}/${n}x${suffix}.${ext} ${n}x`)
    .join(', ');
}

/** Emotes found to have no first-frame file (`provider:id`). The CDNs publish
 *  one only for emotes that animate, so an emote whose animation is unknown
 *  (a personal or partner-room emote, a set not loaded yet) answers 404 for
 *  it; once seen, it is drawn live everywhere and never asked again. Shared by
 *  chat rows and the emote panel. */
const noStillFrame = new Set<string>();
const NO_STILL_FRAME_MAX = 5000;

export function hasNoStillFrame(provider: string | undefined, id: string | undefined): boolean {
  return !!provider && !!id && noStillFrame.has(`${provider}:${id}`);
}

export function markNoStillFrame(provider: string | undefined, id: string | undefined): void {
  if (!provider || !id) return;
  if (noStillFrame.size >= NO_STILL_FRAME_MAX) noStillFrame.clear();
  noStillFrame.add(`${provider}:${id}`);
}

/** srcSet for a 7TV emote in static mode, mirroring the animated one. */
export function static7tvSrcSet(emoteId: string): string {
  return sevenTvSrcSet(emoteId, 'avif', true);
}
