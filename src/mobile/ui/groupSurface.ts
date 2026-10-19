// Which surface a grouped row sits on (see GroupedList), kept apart from the
// components so the hook can be shared by rows built elsewhere.
import { createContext, useContext } from 'react';

/** `card`: a solid card of its own, for a page. `sheet`: straight on a sheet
 *  that is already solid enough to read on, with no card and no hairlines. */
export type GroupSurface = 'card' | 'sheet';

export const GroupSurfaceContext = createContext<GroupSurface>('card');

/** The row classes for the surface a row sits on: on a card the row bleeds
 *  over the card's 16px padding and draws the hairline above it; on a sheet
 *  it keeps the sheet's own 8px inset. For rows that need their own markup (a
 *  settings row with a control) but the same frame. */
export function useGroupRowFrame(): string {
  const surface = useContext(GroupSurfaceContext);
  return surface === 'card' ? 'sn-group-row -mx-4 px-4 w-[calc(100%+2rem)]' : 'px-2 w-full';
}

export function useGroupSurface(): GroupSurface {
  return useContext(GroupSurfaceContext);
}
