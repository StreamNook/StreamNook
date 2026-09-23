import { PROVIDER_WATCH, type ProviderId } from '../types/providers';

/**
 * How a stream card presents its thumbnail.
 *
 * The LAYOUT owns the well's shape; the source only chooses how it fills it.
 *
 * - `cover`: a landscape well, cropped to fill. The long-standing card, and what
 *   every landscape platform keeps byte for byte.
 * - `portrait`: a 9:16 well, for a grid that shows only portrait-first streams.
 *   The medium's own shape, which is the reason such a grid exists.
 * - `pillar`: a portrait picture inside a LANDSCAPE well, shown whole over a
 *   blurred copy of itself. For the mixed view, where cards of different shapes
 *   would otherwise make every row as tall as its tallest card.
 */
export type ThumbFit = 'cover' | 'portrait' | 'pillar';

export function thumbFitFor(provider: ProviderId, portraitGrid: boolean): ThumbFit {
  if (PROVIDER_WATCH[provider]?.thumbAspect !== 'portrait') return 'cover';
  return portraitGrid ? 'portrait' : 'pillar';
}

/** Whether a grid scoped to `provider` should use portrait wells. */
export function isPortraitGrid(provider: ProviderId | 'all' | null | undefined): boolean {
  if (!provider || provider === 'all') return false;
  return PROVIDER_WATCH[provider]?.thumbAspect === 'portrait';
}
