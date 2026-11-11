// The phone's side chat in landscape (beside the video, or over it): how
// narrow it may go, and when its message bar folds to a button (SN-268).
//
// A phone on its side is about 2.2 times as wide as it is tall, so a 16:9
// picture at full height leaves roughly a sixth of the width beside it. The
// column could not go under 240 before, which kept the picture short of full
// height on most phones; 140 lets it fill the height with chat still readable
// beside it. The desktop's full screen overlay keeps its own floor of 240.

/** The narrowest the phone's side chat goes, in px. */
export const PHONE_SIDE_CHAT_MIN = 140;
/** The desktop full screen overlay's narrowest. */
export const DESKTOP_SIDE_CHAT_MIN = 240;
/** Below this the message bar has no room for a usable field and folds to a button. */
export const COMPOSER_FOLD_BELOW = 240;
/** How wide the column opens while a folded bar is being written in. */
export const WRITING_WIDTH = 320;

/** The floor for the full screen chat width on this kind of device. */
export function sideChatMin(phone: boolean): number {
  return phone ? PHONE_SIDE_CHAT_MIN : DESKTOP_SIDE_CHAT_MIN;
}

/** The column at `width` folds its message bar. */
export function composerFolds(width: number): boolean {
  return width < COMPOSER_FOLD_BELOW;
}

/** The column's width on screen: the saved width, or, while a folded bar is
 *  open for writing, wide enough to write in (never past `max`, never
 *  narrower than the saved width). */
export function sideChatShownWidth(width: number, max: number, writing: boolean): number {
  if (!writing || !composerFolds(width)) return width;
  return Math.max(width, Math.min(WRITING_WIDTH, max));
}
