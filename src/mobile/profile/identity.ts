// Small rules the identity surfaces share (IdentityParts.tsx draws them).
import {
  COSMETIC_ASSET_BY_SLUG,
  DEFAULT_COSMETIC_SLUG,
  resolveCosmeticAsset,
} from '../../components/cosmeticAssets';
import { getAtmosphere, type Atmosphere } from '../../services/atmospheres';
import { houseSlug } from './memberIcons';
import { streamnookArtSrc } from '../../services/streamnookArt';
import type {
  ExternalPickerView,
  SnMemberBadge,
  StreamNookPickerView,
} from '../../types/profileView';

/** 7TV's own blue, for its mark and its Connect pill only. */
export const SEVENTV_BLUE = '#29b6f6';
/** The text on a 7TV-blue pill: 7TV's deep navy, readable on the blue. */
export const SEVENTV_INK = '#05212e';

/** A name's color, Twitch purple when it has none. */
export const nameColorOr = (hex?: string | null): string => hex || '#9147ff';

/**
 * Type size for a name under the art. Twitch caps names at 25 characters, so a
 * size step keeps even the longest to two lines rather than cutting your own
 * name short.
 */
export function identityNameSize(name: string): number {
  const n = [...name].length;
  if (n >= 21) return 19;
  if (n >= 17) return 22;
  if (n >= 13) return 24;
  return 26;
}

export type Brand = 'twitch' | 'kick' | 'seventv' | 'streamnook';

/** The brand a group title or provider string names, when a mark can say it. */
export function brandFor(name: string | null | undefined): Brand | null {
  switch ((name ?? '').trim().toLowerCase()) {
    case 'twitch':
      return 'twitch';
    case 'kick':
      return 'kick';
    case '7tv':
    case 'seventv':
      return 'seventv';
    case 'streamnook':
      return 'streamnook';
    default:
      return null;
  }
}

/** The StreamNook badge a member wears, as an image: bundled house art first,
 *  then the catalog's own URL, then the plain mark. */
export function medalUrl(sn: SnMemberBadge | null | undefined): string | null {
  if (!sn) return null;
  return (
    resolveCosmeticAsset({ slug: sn.slug ?? DEFAULT_COSMETIC_SLUG, asset_path: sn.image_url }) ??
    COSMETIC_ASSET_BY_SLUG[DEFAULT_COSMETIC_SLUG] ??
    null
  );
}

/** A badge image a renderer can load: bundled house art for a
 *  `streamnook-house:<slug>` URL, Rust's disk copy of StreamNook CDN art (as
 *  the chat rows draw it), any other URL as it is. */
export function badgeArt(url: string): string {
  const slug = houseSlug(url);
  return (slug && COSMETIC_ASSET_BY_SLUG[slug]) || streamnookArtSrc(url);
}

/** The badges you wear, in chat's order: Twitch, 7TV, the chat apps,
 *  StreamNook last. */
export function wornPreviewBadges(
  sn: StreamNookPickerView | null,
  ext: ExternalPickerView | null,
): string[] {
  const snWorn = sn?.badges.find((b) => b.worn);
  return [
    ext?.twitch_selectable.find((b) => b.is_selected)?.image_url,
    ext?.seventv_badges.find((b) => b.worn)?.badge.url,
    ...(ext?.chat_clients ?? []).map((b) => b.url),
    snWorn ? resolveCosmeticAsset({ slug: snWorn.slug, asset_path: snWorn.image_url ?? null }, { chatSize: true }) : null,
  ].filter((u): u is string => !!u);
}

/** The theme shown in chat (for the preview's wash), or null while none is. */
export function chatAtmosphereId(sn: StreamNookPickerView | null): string | null {
  return sn && sn.show_chat_toggle && sn.chat_atmosphere ? sn.theme : null;
}

/** A theme id's look for the identity glow; the Default and paint looks have none. */
export function glowAtmosphere(theme: string | null | undefined): Atmosphere | null {
  if (!theme || theme === 'tier' || theme === 'paint') return null;
  return getAtmosphere(theme);
}
