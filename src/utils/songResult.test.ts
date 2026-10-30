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

import { dismissSongResult, publishSongResult, songResultFor, subscribeSongResults } from './songResult';

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
