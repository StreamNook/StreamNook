// The width the message box reserves for the room-mode icons
// (RoomModeIndicator), handed to the field as CSS variables that globals.css
// turns into `--sn-room-pad`.
import type React from 'react';
import type { RoomModeView } from '../../stores/chatConnectionStore';

export const ICON_PX = 13;
export const GAP_PX = 4;
/** The icons' distance from the field's right edge. */
export const EDGE_PX = 8;
/** Clear space between the typed text and the first icon. */
export const CLEAR_PX = 6;
/** The "+N" beside the one icon a narrow box keeps. */
export const COUNT_PX = 13;

/** Right padding the field reserves for `count` mode icons: every icon when
 *  wide, the first plus a "+N" count when narrow. */
export function roomModesPad(count: number): { wide: number; narrow: number } {
  if (count <= 0) return { wide: 12, narrow: 12 };
  const wide = EDGE_PX + count * ICON_PX + (count - 1) * GAP_PX + CLEAR_PX;
  const narrow = EDGE_PX + ICON_PX + (count > 1 ? GAP_PX + COUNT_PX : 0) + CLEAR_PX;
  return { wide, narrow };
}

/** The inline style that hands the field its reserved width. */
export function roomModesFieldStyle(view: RoomModeView | null): React.CSSProperties | undefined {
  if (!view || view.modes.length === 0) return undefined;
  const pad = roomModesPad(view.modes.length);
  return {
    '--sn-room-pad-wide': `${pad.wide}px`,
    '--sn-room-pad-narrow': `${pad.narrow}px`,
  } as React.CSSProperties;
}
