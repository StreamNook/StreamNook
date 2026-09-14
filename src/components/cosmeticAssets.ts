// Slug -> Vite-resolved asset URL for cosmetic badges.
//
// The Supabase `cosmetics.asset_path` column is the source of truth for which
// file goes with which slug, but the actual bundling of the image into the
// build happens here so Vite can fingerprint the URL. When a new cosmetic is
// added to the catalog, drop the asset under src/assets and add a line here.

import defaultBadge from '../assets/streamnook-logo-128.webp';
import supporterBadge from '../assets/streamnook-badge-gold-128.webp';
import subscriberBadge from '../assets/streamnook-badge-gold-animated.webp';
import subscriberBadgeChat from '../assets/streamnook-badge-gold-animated-64.webp';
import { resolveManifestAsset, type AssetManifest } from '../services/cosmetics/types';

export const COSMETIC_ASSET_BY_SLUG: Record<string, string> = {
  'streamnook-default': defaultBadge,
  'streamnook-supporter': supporterBadge,
  'streamnook-subscriber': subscriberBadge,
};

// 64px variants for surfaces that render the badge at 64 CSS px or smaller
// (chat rows, pickers, unlock notes, the badges grid). The animated subscriber
// badge is the only one that needs a variant: the 128px original stays on the
// large renders (badge detail modal, the scalable overlay chat), where the
// downscale is visibly blurry when stretched.
const COSMETIC_CHAT_VARIANT_BY_SLUG: Record<string, string> = {
  'streamnook-subscriber': subscriberBadgeChat,
};

/** Same shape as COSMETIC_ASSET_BY_SLUG with chat-size variants substituted
 *  where one exists; for direct-map consumers rendering at 64px or below. */
export const COSMETIC_ASSET_CHAT_BY_SLUG: Record<string, string> = {
  ...COSMETIC_ASSET_BY_SLUG,
  ...COSMETIC_CHAT_VARIANT_BY_SLUG,
};

/**
 * Resolve a cosmetic's displayable image URL. Order: the bundled asset if we
 * ship one for this slug (so the gold trio renders identically), then the
 * asset manifest (when a definition carries one), then the catalog's cloud
 * `asset_path` (an R2 URL on cdn.streamnook.app). Returns null when none is
 * usable, so callers can skip a cosmetic they can't render. This is what lets a
 * cloud-served badge (a DB row + an upload, no desktop release) show in chat.
 */
export function resolveCosmeticAsset(
  cosmetic:
    | { slug: string; asset_path?: string | null; asset_manifest?: AssetManifest | null }
    | null
    | undefined,
  opts: { reducedMotion?: boolean; chatSize?: boolean } = {},
): string | null {
  if (!cosmetic) return null;
  const bundled = opts.chatSize
    ? COSMETIC_ASSET_CHAT_BY_SLUG[cosmetic.slug]
    : COSMETIC_ASSET_BY_SLUG[cosmetic.slug];
  if (bundled) return bundled;
  const fromManifest = resolveManifestAsset(cosmetic.asset_manifest, opts);
  if (fromManifest) return fromManifest;
  const path = cosmetic.asset_path;
  return path && /^https?:\/\//.test(path) ? path : null;
}
