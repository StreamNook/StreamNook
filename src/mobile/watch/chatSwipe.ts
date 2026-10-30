// The landscape chat swipe (SN-30): a sideways swipe across the picture brings
// chat in from its side column and puts it away again. Pure, so both phones
// test the same numbers (src/mobile/phoneGestureCases.json; the iPhone's twin
// is SNKit ChatSwipe).

export type ChatSide = 'left' | 'right';

export const SWIPE_MIN = 48;
export const SWIPE_RATIO = 1.5;
/** CSS px per millisecond. */
export const FLING_SPEED = 0.5;
export const FLING_MIN = 24;

/** What a released sideways swipe does: away from chat's side opens it, toward
 *  it closes it. 48 px, or a flick, and clearly more sideways than not. */
export function chatSwipeIntent(s: {
  dx: number;
  dy: number;
  vx: number;
  side: ChatSide;
  open: boolean;
}): 'open' | 'close' | null {
  const ax = Math.abs(s.dx);
  const ay = Math.abs(s.dy);
  const travelled = ax >= SWIPE_MIN && ax >= SWIPE_RATIO * ay;
  const flicked = Math.abs(s.vx) >= FLING_SPEED && ax >= FLING_MIN && ax > ay;
  if (!travelled && !flicked) return null;
  const towardChat = s.side === 'right' ? s.dx > 0 : s.dx < 0;
  if (!s.open && !towardChat) return 'open';
  if (s.open && towardChat) return 'close';
  return null;
}

/** Where chat comes in from as a side column, or null where it does not
 *  (portrait, a large screen's stacked arrangement, a large portrait screen). */
export function chatSwipeSide(c: {
  sideBySide: boolean;
  largeScreen: boolean;
  largeToggle: boolean;
  largeChatHidden: boolean;
  twoColumns: boolean;
  columnsOnReturn: boolean;
  overlaySide: ChatSide;
}): ChatSide | null {
  if (!c.sideBySide) return null;
  if (!c.largeScreen) return c.overlaySide;
  if (!c.largeToggle) return null;
  return (c.largeChatHidden ? c.columnsOnReturn : c.twoColumns) ? 'right' : null;
}

/** The screen edge to claim from Android's Back gesture: chat's side while the
 *  immersive player is up (bars hidden, where Android claims a whole edge),
 *  chat is away and nothing else takes Back first. */
export function backClaimSide(c: {
  immersive: boolean;
  side: ChatSide | null;
  chatOpen: boolean;
  backTaken: boolean;
}): ChatSide | null {
  return c.immersive && c.side && !c.chatOpen && !c.backTaken ? c.side : null;
}
