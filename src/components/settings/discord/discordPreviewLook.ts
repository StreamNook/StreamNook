// Which picture the Discord preview draws: Rust decides the look
// (services/discord_look.rs); this only picks the moving or the still member
// of a pair, by Interface > Motion and by hover.

import type { MotionMode } from '../../../types';

export interface LookPair {
  animated: string;
  still: string;
}

/** The picture to draw: the moving one only when it may move. */
export function pick(pair: LookPair | null | undefined, animate: boolean): string | null {
  if (!pair) return null;
  return animate ? pair.animated : pair.still;
}

/** The profile card moves only with Motion on Full (the default). */
export function animateCard(motion: MotionMode | undefined): boolean {
  return (motion ?? 'full') === 'full';
}

/** The member-list row moves only while hovered, as Discord's list does. */
export function animateRow(motion: MotionMode | undefined, hovered: boolean): boolean {
  return animateCard(motion) && hovered;
}

/** The nameplate's moving art is a WebM, unproven in macOS's web view: macOS draws its still. */
export function plateVideo(video: string | null | undefined, animate: boolean, isMac: boolean): string | null {
  return animate && !isMac && video ? video : null;
}
