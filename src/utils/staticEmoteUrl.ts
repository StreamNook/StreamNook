// Static (first-frame) variant of an animated emote URL, per provider CDN.
// Used by the "Animate emotes" setting: never, or only while the row is
// hovered. Pure URL rewriting; the CDNs publish the static files themselves.
//
//   7TV     .../emote/<id>/1x.avif        -> .../emote/<id>/1x_static.avif
//   BTTV    .../emote/<id>/1x             -> .../emote/<id>/static/1x
//   FFZ     .../emote/<id>/animated/1     -> .../emote/<id>/1
//   Twitch  .../v2/<id>/default/dark/2.0  -> .../v2/<id>/static/dark/2.0
//
// Anything else (disk-cached asset:// files, unknown hosts) is returned as is.

export function staticEmoteUrl(url: string): string {
  if (!url || url.startsWith('asset://') || url.includes('asset.localhost')) return url;
  if (url.includes('cdn.7tv.app/emote/')) {
    if (url.includes('_static.')) return url;
    return url.replace(/\/(\dx)\.(avif|webp|png|gif)(\?.*)?$/, '/$1_static.$2$3');
  }
  if (url.includes('cdn.betterttv.net/emote/')) {
    if (url.includes('/static/')) return url;
    return url.replace(/\/emote\/([^/]+)\/(\dx)$/, '/emote/$1/static/$2');
  }
  if (url.includes('cdn.frankerfacez.com/emote/')) {
    return url.replace(/\/emote\/([^/]+)\/animated\/(\d)/, '/emote/$1/$2');
  }
  if (url.includes('static-cdn.jtvnw.net/emoticons/')) {
    return url.replace('/default/', '/static/');
  }
  return url;
}

/** srcSet for a 7TV emote in static mode, mirroring the animated one. */
export function static7tvSrcSet(emoteId: string): string {
  return [1, 2, 3, 4]
    .map((n) => `https://cdn.7tv.app/emote/${emoteId}/${n}x_static.avif ${n}x`)
    .join(', ');
}
