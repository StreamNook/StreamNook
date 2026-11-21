// What the phone's chat screens tell Rust about their pin surface (SN-284), as
// pure rules the screens call and the tests read. A surface that is not in
// front must say so: Rust ranks the phone's watch screen before its Chats
// screen, so a watch screen that reports itself in front while it draws
// nothing would take every pin and show it to nobody.

/** The watch screen's chat is out of sight: no stream at all (the layer renders
 *  nothing while its hooks still run), the mini box, PiP, or a landscape with
 *  chat away (the column's class is `hidden`). */
export function watchChatOutOfSight(i: { show: boolean; mini: boolean; chatClass: string }): boolean {
  return !i.show || i.mini || i.chatClass === 'hidden';
}

export interface WatchPinInputs {
  /** The watch layer is up (there is a stream, or one is loading). */
  show: boolean;
  mini: boolean;
  pip: boolean;
  /** The class the chat column wears; `hidden` while it is not drawn. */
  chatClass: string;
  /** Landscape chat floating over the video. */
  chatOverlay: boolean;
  /** The tall layout's chat states. */
  verticalLower: boolean;
  verticalGrown: boolean;
  verticalCleared: boolean;
  sideBySide: boolean;
}

export interface WatchPinSurface {
  /** The chat column is not drawn. */
  chatOutOfSight: boolean;
  /** The pins surface is out of sight: the column is not drawn and nothing else
   *  draws the pins in its place. */
  pinsOutOfSight: boolean;
  /** The tall chat cleared: its pins show as strips over the video. */
  clearedOverVideo: boolean;
  mode: string;
  stripsOnly: boolean;
  overVideo: boolean;
}

/** The watch screen's pin surface: the lower third and the cleared chat both
 *  show strips only, over the video (a tap on one grows the chat). */
export function watchPinSurface(i: WatchPinInputs): WatchPinSurface {
  const chatOutOfSight = watchChatOutOfSight(i);
  const clearedOverVideo = i.verticalCleared && i.show && !i.mini && !i.pip;
  return {
    chatOutOfSight,
    pinsOutOfSight: chatOutOfSight && !clearedOverVideo,
    clearedOverVideo,
    mode: i.chatOverlay || i.verticalCleared
      ? 'over-video'
      : i.verticalLower || i.verticalGrown
        ? 'vertical'
        : i.sideBySide
          ? 'landscape'
          : 'stacked',
    stripsOnly: i.verticalLower || i.verticalCleared,
    overVideo: i.chatOverlay || i.verticalLower || i.verticalCleared,
  };
}

/** The Chats screen's chat is out of sight: another tab is in front, the full
 *  player is up over it, or the app is unseen. */
export function chatsChatOutOfSight(i: { shown: boolean; layerUp: boolean; idle: boolean }): boolean {
  return !i.shown || i.layerUp || i.idle;
}
