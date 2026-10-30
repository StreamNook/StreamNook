// The Songs popover's placement and the checks the player makes on it. Pure,
// so it is tested without a page (SongsFlyout.tsx draws it).

/** The popover's root carries this, so the player can tell it is open
 *  although it is portaled out of the player. */
export const SONGS_FLYOUT_ATTR = 'data-songs-flyout';

export const FLYOUT_WIDTH = 400;
/** Below the split button. */
const GAP = 6;
/** Clear of the window's sides. */
const SIDE = 8;
/** Clear of the window's foot. */
const FOOT = 12;

export interface Anchor {
  left: number;
  right: number;
  top: number;
  bottom: number;
}

export interface Placement {
  top: number;
  left: number;
  width: number;
  /** The popover scrolls its recent songs inside this. */
  maxHeight: number;
}

/**
 * Where the popover opens for the split button at `anchor` (its client rect)
 * in a `w` x `h` window: right-aligned to the button and just below it, kept
 * inside the window on both sides, as wide as the window allows and no
 * taller than the room left to its foot.
 */
export function placeFlyout(anchor: Anchor, viewport: { w: number; h: number }): Placement {
  const width = Math.max(0, Math.min(FLYOUT_WIDTH, viewport.w - SIDE * 2));
  const left = Math.max(SIDE, Math.min(anchor.right - width, viewport.w - SIDE - width));
  const top = Math.round(anchor.bottom + GAP);
  return { top, left: Math.round(left), width: Math.round(width), maxHeight: Math.max(0, Math.floor(viewport.h - top - FOOT)) };
}

/** Whether the Songs popover is open in this page (checked on the document,
 *  since it is portaled to its body). */
export function songsFlyoutOpen(doc: Pick<ParentNode, 'querySelector'> | null | undefined): boolean {
  return !!doc?.querySelector(`[${SONGS_FLYOUT_ATTR}]`);
}

/**
 * The popover's stacking: in the player's full screen, the title bar's menu
 * layer (over the full-screen title and badge overlays, under the tooltips);
 * in the window, over the player, lifted like the other page-level menus.
 */
export function flyoutZIndex(fullscreen: boolean): string {
  return fullscreen ? 'var(--sn-z-titlebar-menu)' : 'calc(var(--sn-z-over-player-lift, 0) + 9999)';
}
