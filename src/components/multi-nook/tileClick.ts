/** What a click on a MultiNook tile does. */
export type TileClickAction = 'ignore' | 'focus' | 'unfocus-later' | 'toggle-play-later';

export interface TileClickInput {
  /** The click landed on a button, the control bar or a tile control. */
  onControl: boolean;
  /** The tile fills the whole grid area. */
  isMaximized: boolean;
  /** The tile is the one whose audio is on. */
  isFocused: boolean;
  /** The click count of the event (2 is the second click of a double-click). */
  detail: number;
  /** The Click to pause setting. Absent means on. */
  clickToPause?: boolean;
}

/**
 * A control, or the second click of a double-click (the capture-phase
 * double-click handler owns that one), does nothing. A maximized tile has no
 * focus to change, so a click toggles play and pause after a short hold that a
 * double-click can cancel, unless Click to pause is off. Otherwise a click
 * focuses an unfocused tile at once and unfocuses a focused tile after a hold.
 */
export function tileClickAction({ onControl, isMaximized, isFocused, detail, clickToPause = true }: TileClickInput): TileClickAction {
  if (onControl || detail >= 2) return 'ignore';
  if (isMaximized) return clickToPause ? 'toggle-play-later' : 'ignore';
  return isFocused ? 'unfocus-later' : 'focus';
}
