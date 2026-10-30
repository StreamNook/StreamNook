// Where the listening strip shows, and what its Cancel does. Rooms are compared
// only in the chat-slice key space (a Twitch login, or a provider's composite
// key), folded to lowercase on both sides: songListening folds the room it is
// given, and a widget's room is folded here.

import { useAppStore } from '../../stores/AppStore';
import { openExternal } from '../../utils/openExternal';
import { cancelListening, listensRemotely, requestRemoteIdentify, type Listening } from '../../utils/songListening';

/** The strip shows in the chat whose room the answer goes to, and nowhere
 *  else (two widgets on that room both show it). */
export function stripShows(listening: Pick<Listening, 'room'> | null, room: string | null | undefined): boolean {
  return !!listening?.room && !!room && listening.room === room.toLowerCase();
}

/** "Try again" after a miss: listen once more for `room`, here when this
 *  window holds the player, else through the main window's (as /song does). */
export async function identifyAgain(room: string): Promise<void> {
  if (listensRemotely()) {
    await requestRemoteIdentify(room);
    return;
  }
  const { recognizeNowPlaying, announceSong } = await import('../../utils/songId');
  const result = await recognizeNowPlaying(null, { room });
  announceSong(result, room);
}

/** A song link opened from the answer card, with a word when the system
 *  would not open it. */
export function openSongLink(url: string): void {
  void openExternal(url).then((opened) => {
    if (!opened) useAppStore.getState().addToast("Couldn't open that link", 'error');
  });
}

/**
 * The one floating shape the listening pill, the finding pill and the answers
 * share: listening, finding and the answer are one object whose contents
 * change. 60 px tall with an 18 px radius, on the flyout's solid base (no blur
 * over the video column).
 */
export const ANSWER_SHELL =
  'glass-flyout no-live-blur relative h-[60px] w-full overflow-hidden !rounded-[18px] !p-0';

/** One state's contents in the shape: a row 10 px in from every edge, with a
 *  40 px slot on the left (the bars, the cover, an icon). */
export const ANSWER_ROW = 'absolute inset-0 flex items-center gap-3 p-[10px]';

/**
 * The shape's lift off the chat: a hairline in the theme's subtle border, a
 * quiet, neutral still shadow (lighter on a light theme), and under both a
 * soft clearing in the chat's own color. The shape floats over the chat's
 * lines, which start a little left of it, so without the clearing the first
 * letters of a name show beside it and its rounded corners uncover more.
 */
export function answerShadow(light: boolean): { boxShadow: string } {
  const drop = light ? '0 6px 16px -8px rgba(0, 0, 0, 0.2)' : '0 6px 18px -8px rgba(0, 0, 0, 0.45)';
  const clearing = '0 0 8px 8px var(--color-background)';
  return { boxShadow: `inset 0 0 0 1px var(--color-border-subtle), ${drop}, ${clearing}` };
}

/** How long an answer shows over the composer before it folds away: a song
 *  long enough to catch and act on, a miss or an error briefly. */
export const RESULT_MS = { match: 8000, other: 4000 } as const;

/** Cancel only while listening: once the clip is sent the song is filed
 *  within a second. */
export function stripCancel(
  listening: Pick<Listening, 'phase'> | null,
  cancel: () => void = cancelListening,
): (() => void) | null {
  return listening?.phase === 'listening' ? cancel : null;
}
