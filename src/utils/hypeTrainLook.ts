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

/** Whether the bar names the train ("Heist Train · Level 3"): any train but a
 *  plain regular one, whose bar just says "Level 3". */
export function namesTrain(look: HypeTrainLookData | null | undefined): boolean {
  return !!look && !!look.bar_name && (look.kind !== 'regular' || !!look.theme || look.shared);
}

/** The type's colors as CSS variables on the card (`[data-themed]`): the
 *  fill's flowing gradient (and its glow and sparks), the accent for the type
 *  words, the ring and the level-up words, and a faint tint of the fill under
 *  the glass. The card itself stays the app's glaze, words in the theme's ink. */
export function paletteStyle(palette: HypeTrainPalette | null): CSSProperties {
  if (!palette || palette.fill.length === 0) return {};
  const stops = palette.fill.length > 1 ? palette.fill : [palette.fill[0], palette.fill[0]];
  return {
    '--hype-fill-gradient': `linear-gradient(90deg, ${stops.join(', ')})`,
    '--hype-tint': `linear-gradient(90deg, ${stops.join(', ')})`,
    '--hype-accent': palette.accent,
    '--hype-spark': palette.accent,
  } as CSSProperties;
}

/** The level-up confetti in the type's colors: its fill stops and its accent
 *  (a heist throws its blue, pink and orange). Empty keeps the rainbow. */
export function confettiColors(palette: HypeTrainPalette | null): string[] {
  if (!palette) return [];
  return [...new Set([...palette.fill, palette.accent].map((c) => c.toLowerCase()))];
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
