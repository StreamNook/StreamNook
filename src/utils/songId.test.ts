import { beforeEach, describe, expect, it, vi } from 'vitest';

const inject = vi.fn();
// Keyed the way the chat store keys its slices: lowercased.
const rooms = new Map<string, unknown>();
vi.mock('../stores/chatConnectionStore', () => ({
  injectSystemMessage: (...args: unknown[]) => inject(...args),
  systemSourceFor: () => undefined,
  useChatConnectionStore: { getState: () => ({ channels: rooms }) },
}));
vi.mock('../stores/AppStore', () => ({ useAppStore: { getState: () => ({ settings: {}, currentStream: null }) } }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

import { announceSong, type RecognizeResult } from './songId';

const match: RecognizeResult = {
  status: 'match',
  song: { title: 'One', artist: 'A', album: null, album_art: null, shazam_url: null, song_link: null, providers: [], key: 'k' },
};

describe('where a song answer is posted', () => {
  beforeEach(() => {
    inject.mockClear();
    rooms.clear();
  });

  it('posts in the named room whatever case its key comes in', () => {
    rooms.set('xqc', {});
    rooms.set('youtube:ucabc', {});
    announceSong(match, 'XQC');
    announceSong(match, 'youtube:UCAbc');
    expect(inject.mock.calls.map((c) => c[0])).toEqual(['XQC', 'youtube:UCAbc']);
    expect(inject.mock.calls[0][2]).toEqual(match.status === 'match' ? match.song : null);
  });

  it('falls back to the page event when the room is not held here or none is named', () => {
    const dispatchEvent = vi.fn();
    vi.stubGlobal('window', { dispatchEvent });
    announceSong(match, 'kick:gone');
    announceSong({ status: 'no-match' }, null);
    expect(inject).not.toHaveBeenCalled();
    expect(dispatchEvent).toHaveBeenCalledTimes(2);
    vi.unstubAllGlobals();
  });
});
