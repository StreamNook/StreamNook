// Helpers for reasoning about Streamlink quality strings like "1080p60", "480p",
// "audio_only", "best", "worst". Used to decide when a closest-match fallback
// is meaningfully different from what the user asked for.

import { IS_MOBILE } from './platform';

const SYMBOLIC_QUALITIES = new Set(['auto', 'best', 'worst', 'audio_only']);

export function parseQualityHeight(q: string): number | null {
  const m = q.trim().match(/^(\d+)/);
  return m ? parseInt(m[1], 10) : null;
}

export function parseQualityFps(q: string): number | null {
  const m = q.trim().toLowerCase().match(/^\d+p(\d+)/);
  return m ? parseInt(m[1], 10) : null;
}

export function isSymbolicQuality(q: string): boolean {
  return SYMBOLIC_QUALITIES.has(q.trim().toLowerCase());
}

/**
 * Two quality strings refer to the same playable stream if they're either
 * the literal same string, or share the same resolution with compatible
 * framerates (one side has no explicit fps, or both fps values match).
 *
 * Examples of equivalent pairs: "480" ~ "480p" ~ "480p30" when the channel only
 * exposes one of those names; "best" ~ "best".
 *
 * Examples of NOT equivalent: "480p60" vs "480p30" (different framerate, real
 * difference in bitrate); "best" vs "1080p60" (symbolic name only matches itself
 * — "best" is "whatever's highest" by definition, not a specific resolution).
 */
export function qualitiesEquivalent(a: string, b: string): boolean {
  const an = a.trim().toLowerCase();
  const bn = b.trim().toLowerCase();
  if (an === bn) return true;

  // Symbolic qualities only match their own literal name. "best" stays "best"
  // even when Streamlink resolves it to 1080p60 under the hood.
  if (SYMBOLIC_QUALITIES.has(an) || SYMBOLIC_QUALITIES.has(bn)) return false;

  const ah = parseQualityHeight(an);
  const bh = parseQualityHeight(bn);
  if (ah === null || bh === null) return false;
  if (ah !== bh) return false;

  const af = parseQualityFps(an);
  const bf = parseQualityFps(bn);
  return af === null || bf === null || af === bf;
}

/**
 * How a quality reads in the player menus. "auto" follows the connection, so it
 * names the rendition it plays right now when Rust has said (`playback-adapt`).
 * On the phones "best" is their Auto; on the desktop it is the fixed source.
 */
export function qualityLabel(quality: string, autoPlaying?: string | null, mobile: boolean = IS_MOBILE): string {
  const q = quality.trim().toLowerCase();
  if (q === 'auto' || (q === 'best' && mobile)) return autoPlaying ? `Auto (${autoPlaying})` : 'Auto';
  if (q === 'best' || q === 'source') return 'Source';
  return quality.charAt(0).toUpperCase() + quality.slice(1);
}
