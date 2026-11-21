import type { CSSProperties } from 'react';
import type { HypeTrainLookData, HypeTrainPalette } from '../types';

// How the hype train bar, its confetti and the stream-card badges draw a
// train's kind. Rust decides the kind, the words and the palette
// (`services/hype_train_kind.rs`); these only turn them into CSS and colors.
// The iPhone twin is SNKit `HypeTrainLook`.

/** The palette a train is painted in; null keeps the app's own rainbow (a
 *  regular train, or an answer from before kinds existed). */
export function trainPalette(train: { look?: HypeTrainLookData | null }): HypeTrainPalette | null {
  return train.look?.palette ?? null;
}

/** The card's look name for CSS (`data-look`): the theme when there is one
 *  (`heist`), else the kind. */
export function lookName(look: HypeTrainLookData | null | undefined): string {
  return look?.theme ?? look?.kind ?? 'regular';
}

/** Whether the bar names the train ("Heist", "Shared"): any train but a plain
 *  regular one, whose bar just says its level. */
export function namesTrain(look: HypeTrainLookData | null | undefined): boolean {
  return !!look && !!look.bar_name && (look.kind !== 'regular' || !!look.theme || look.shared);
}

/** The card's palette as CSS variables: the gradient, the accent and the fill,
 *  and white text, since every palette is a dark, saturated card. */
export function paletteStyle(palette: HypeTrainPalette | null): CSSProperties {
  if (!palette) return {};
  const stops = palette.background.length > 1 ? palette.background : [palette.background[0] ?? palette.accent, palette.background[0] ?? palette.accent];
  return {
    // The glaze's own rim and glow light in the kind's color, so the card
    // stays the glaze, tinted, rather than a painted panel.
    '--color-accent': palette.badge,
    '--hype-bg': `linear-gradient(90deg, ${stops.join(', ')})`,
    '--hype-accent': palette.accent,
    '--hype-fill': palette.fill,
    '--color-text-primary': '#ffffff',
    '--color-text-secondary': 'rgba(255, 255, 255, 0.82)',
  } as CSSProperties;
}

/** The level-up confetti in the palette's colors: its gradient and its accent
 *  (a heist throws its three colors and gold). Empty keeps the rainbow. */
export function confettiColors(palette: HypeTrainPalette | null): string[] {
  if (!palette) return [];
  return [...new Set([...palette.background, palette.accent, palette.fill].map((c) => c.toLowerCase()))];
}

/** The stream-card chip: its color (the glaze's accent) and its word. */
export function badgeLook(look: HypeTrainLookData | null | undefined, level: number): { color: string | null; word: string } {
  const word = look?.short_name ? `${look.short_name.toUpperCase()} ${level}` : `LVL ${level}`;
  return { color: look?.palette?.badge ?? null, word };
}

/** Whether `started_at` (ISO) was within `ms` of now: the train just began,
 *  so its kickoff words play. */
export function justStarted(startedAt: string | null | undefined, now: number, ms = 20_000): boolean {
  if (!startedAt) return false;
  const at = Date.parse(startedAt);
  return Number.isFinite(at) && now - at >= 0 && now - at < ms;
}
