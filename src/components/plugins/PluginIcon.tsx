// One place that decides what image represents a plugin in the marketplace.
// Resolution order:
//   1. an explicit icon the plugin shipped (icon_url)
//   2. for first-party (official) plugins: the StreamNook logo
//   3. otherwise: the author's GitHub avatar (author handle = GitHub username)
//   4. last resort (e.g. an avatar that 404s): a neutral glyph on a tier tint
// So first-party plugins carry the brand automatically and community plugins
// wear their author's face, with no per-plugin icon files required.

import { useState } from 'react';
import { Puzzle } from 'lucide-react';
import type { PluginTier } from '../../types/plugins';
import streamNookLogo from '../../assets/streamnook-logo-128.webp';

// Canonical bevel recipe lives in globals.css as --bevel-tile.
const TILE_BEVEL = 'var(--bevel-tile)';

const TIER_TINT: Record<PluginTier, string> = {
  A: 'rgba(110, 200, 160, 0.16)',
  B: 'rgba(225, 185, 120, 0.16)',
  C: 'rgba(225, 130, 130, 0.16)',
};

/** GitHub serves any user's or org's avatar at https://github.com/<name>.png. */
function githubAvatar(handle: string, size: number): string {
  return `https://github.com/${encodeURIComponent(handle)}.png?size=${size}`;
}

interface PluginIconProps {
  iconUrl?: string | null;
  /** First-party plugin: falls back to the StreamNook logo. */
  official?: boolean;
  /** Author handle (a GitHub username); the third-party fallback is its avatar. */
  author?: string;
  tier: PluginTier;
  /** Container sizing + rounding classes, e.g. "h-11 w-11 rounded-xl". */
  sizeClass: string;
  /** Lucide glyph size for the last-resort fallback. */
  glyphSize: number;
}

export function PluginIcon({
  iconUrl,
  official,
  author,
  tier,
  sizeClass,
  glyphSize,
}: PluginIconProps) {
  const [failed, setFailed] = useState(false);
  const isLogo = !iconUrl && !!official;
  const resolved = iconUrl
    ? iconUrl
    : official
      ? streamNookLogo
      : author
        ? githubAvatar(author, glyphSize * 4)
        : null;
  const showImage = !!resolved && !failed;
  return (
    // A span (with display:flex from the class) so the icon can sit inside
    // text containers like <p> without invalid DOM nesting.
    <span
      className={`flex flex-shrink-0 items-center justify-center overflow-hidden ${sizeClass}`}
      style={showImage ? undefined : { background: TIER_TINT[tier], boxShadow: TILE_BEVEL }}
    >
      {showImage ? (
        <img
          src={resolved}
          alt=""
          // The portrait brand logo is shown whole; square avatars/icons fill.
          className={`h-full w-full ${isLogo ? 'object-contain' : 'object-cover'}`}
          loading="lazy"
          onError={() => setFailed(true)}
        />
      ) : (
        <Puzzle size={glyphSize} strokeWidth={2.25} className="text-textPrimary" />
      )}
    </span>
  );
}

export default PluginIcon;
