// Which sharp file a badge image should draw at its size, if any. The art is made
// from one exact Twitch image (`art.i`), so it is only drawn beside that image.
import type { BadgeArt } from '../../services/badgeStanding';
import { sharpUrls } from './sharpUrls';

/**
 * The sharp CDN URL for an image drawn at `cssPx` on a screen of `dpr`, or null
 * for Twitch's own image: no art, art made from a different image than the one
 * drawn, or a size Twitch's 72 px file already covers. Up to 144 device px gets
 * the 144 px WebP, larger the 288 px master.
 */
export function pickSharp(cssPx: number, dpr: number, drawn: string, art: BadgeArt | null | undefined): string | null {
  if (!art || art.i !== drawn) return null;
  const urls = sharpUrls(art.a);
  if (!urls) return null;
  const devicePx = cssPx * (Number.isFinite(dpr) && dpr > 0 ? dpr : 1);
  if (devicePx <= 72) return null;
  return devicePx <= 144 ? urls.small : urls.large;
}
