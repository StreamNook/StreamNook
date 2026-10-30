// The latest identification's answer for each chat, for the card that shows
// above its composer for a few seconds (ListeningStrip). The song's card still
// lands in the chat as its record (announceSong); this is the moment you can
// catch it. One answer per chat room (a chat-slice key, lowercased): a new one
// replaces it, and a listening starting in that room takes its place at once.

import { getListening, subscribeListening } from './songListening';
import type { RecognizeResult } from './songId';

/** An answer worth showing: a song, nothing found, or why it could not try. */
export type ShownAnswer = Exclude<RecognizeResult, { status: 'canceled' } | { status: 'busy' }>;

export interface SongResult {
  /** Told apart from the room's next answer. */
  id: number;
  room: string;
  result: ShownAnswer;
}

const results = new Map<string, SongResult>();
const watchers = new Set<() => void>();
let nextId = 0;
let watchingListening = false;

function changed(): void {
  for (const watch of [...watchers]) watch();
}

/** A listening starting in a room takes the place of its last answer. */
function onListening(): void {
  const room = getListening()?.room;
  if (room && results.delete(room)) changed();
}

/** Show an answer over `room`'s composer. A room-less answer (the open chat's
 *  fallback) has no shelf to show in. */
export function publishSongResult(result: ShownAnswer, room: string | null): void {
  if (!room) return;
  const key = room.toLowerCase();
  results.set(key, { id: ++nextId, room: key, result });
  changed();
}

/** The answer showing over `room`'s composer, if any. */
export function songResultFor(room: string | null | undefined): SongResult | null {
  return room ? (results.get(room.toLowerCase()) ?? null) : null;
}

/** It has been seen (its time ran out, or it was dismissed). */
export function dismissSongResult(id: number): void {
  for (const [room, r] of results) {
    if (r.id === id) {
      results.delete(room);
      changed();
      return;
    }
  }
}

/** For useSyncExternalStore (its server snapshot is no answer). */
export function subscribeSongResults(cb: () => void): () => void {
  if (!watchingListening) {
    watchingListening = true;
    subscribeListening(onListening);
  }
  watchers.add(cb);
  return () => {
    watchers.delete(cb);
  };
}
