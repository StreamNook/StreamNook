// The StreamNook page plate's fixed look. Its floor, edge and muted text are
// constants and never theme tokens, so Glassiness and a light theme cannot
// change how it reads. Rust (`member_link::PLATE_FLOOR`) lifts the accent to
// read on this same floor, and a test pins the two together.

/** The plate's own floor: near-black at 92%. */
export const PLATE_FLOOR_RGB: readonly [number, number, number] = [14, 14, 18];
export const PLATE_FLOOR_ALPHA = 0.92;
export const PLATE_FLOOR = 'rgba(14,14,18,0.92)';
/** The 1 px edge around the plate. */
export const PLATE_EDGE = 'rgba(255,255,255,0.10)';
/** The prefix text and the arrow. */
export const PLATE_MUTED = '#9aa3ad';
/** The hairline between the link and the copy square. */
export const PLATE_DIVIDER = 'rgba(255,255,255,0.10)';
/** The copy square after a copy. Fixed like the floor it sits on. */
export const PLATE_CHECK = '#4ade80';
export const PLATE_RADIUS = 10;
export const PLATE_MARK = 16;
export const PLATE_TEXT = 13;
export const PLATE_NAME_WEIGHT = 600;
/** Plate height, which is also the copy square's side. */
export const PLATE_SIZE = { desktop: 32, phone: 34 } as const;
/** The empty plate a reserved slot shows is this wide (both phones, and the
 *  desktop the same), so the plate landing changes its content and not its box. */
export const PLATE_RESERVED_WIDTH = 214;
/** Space above the plate, inside the slot, so a collapsing slot takes it too. */
export const PLATE_GAP = 8;
/** How long the copy square shows its check. */
export const PLATE_COPIED_MS = 1400;

/** The accent at 35% as the plate's one-pixel inner top light. */
export function plateTopLight(accent: string): string {
  return `color-mix(in srgb, ${accent} 35%, transparent)`;
}
