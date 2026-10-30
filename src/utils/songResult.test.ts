// The answer over a chat's composer: one per room, replaced by the next, gone
// when seen, and gone at once when a new listening starts in that room.
import { describe, expect, it, vi } from 'vitest';

const listeners = new Set<() => void>();
let current: { room: string | null } | null = null;
vi.mock('./songListening', () => ({
  getListening: () => current,
  subscribeListening: (cb: () => void) => {
    listeners.add(cb);
    return () => listeners.delete(cb);
  },
}));

import {
  ANSWER_MS,
  dismissSongResult,
  holdSongResult,
  publishSongResult,
  songResultFor,
  subscribeSongResults,
} from './songResult';

const miss = { status: 'no-match' as const };

describe('the answer over a chat', () => {
  it('is kept per room, folded, and the next one replaces it', () => {
    const seen = vi.fn();
    const stop = subscribeSongResults(seen);
    publishSongResult(miss, 'XQC');
    const first = songResultFor('xqc');
    expect(first?.result).toEqual(miss);
    expect(songResultFor('kaicenat')).toBeNull();
    publishSongResult({ status: 'error', message: 'No stream is playing.' }, 'xqc');
    expect(songResultFor('xqc')?.id).not.toBe(first?.id);
    expect(seen).toHaveBeenCalledTimes(2);
    stop();
  });

  it('a room-less answer has no shelf to show in', () => {
    publishSongResult(miss, null);
    expect(songResultFor(null)).toBeNull();
  });

  it('goes when it has been seen, and only that one', () => {
    publishSongResult(miss, 'a');
    const shown = songResultFor('a')!;
    dismissSongResult(shown.id + 1000);
    expect(songResultFor('a')).not.toBeNull();
    dismissSongResult(shown.id);
    expect(songResultFor('a')).toBeNull();
  });

  it('is old news past its card’s 8 s: a chat drawn later never shows it', () => {
    expect(ANSWER_MS).toBe(8000);
    publishSongResult(miss, 'late', 1_000);
    expect(songResultFor('late', 1_000 + ANSWER_MS)?.at).toBe(1_000);
    expect(songResultFor('late', 1_000 + ANSWER_MS + 1)).toBeNull();
  });

  it('stays past its time while its card is held, and goes once let go', () => {
    publishSongResult(miss, 'held', 1_000);
    const shown = songResultFor('held', 1_000)!;
    holdSongResult(shown.id, true);
    expect(songResultFor('held', 60_000)?.id).toBe(shown.id);
    holdSongResult(shown.id, false);
    expect(songResultFor('held', 60_000)).toBeNull();
  });

  it('a new listening in the room takes its place at once', () => {
    subscribeSongResults(() => {});
    publishSongResult(miss, 'b');
    publishSongResult(miss, 'c');
    current = { room: 'b' };
    for (const l of listeners) l();
    expect(songResultFor('b')).toBeNull();
    expect(songResultFor('c')).not.toBeNull();
    current = null;
  });
});
