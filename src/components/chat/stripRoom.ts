// Where the listening strip shows, and what its Cancel does. Rooms are compared
// only in the chat-slice key space (a Twitch login, or a provider's composite
// key), folded to lowercase on both sides: songListening folds the room it is
// given, and a widget's room is folded here.

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

/** The listening pill's and the answer card's glass: the flyout's solid base
 *  (no blur over the video column), its hairline, and a soft accent glow
 *  under it, a still shadow. */
export const ANSWER_GLOW = {
  boxShadow:
    'inset 0 1px 0 rgba(255, 255, 255, 0.08), 0 0 0 1px var(--color-border-subtle), 0 10px 24px -12px color-mix(in srgb, var(--color-accent) 60%, transparent)',
} as const;

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
