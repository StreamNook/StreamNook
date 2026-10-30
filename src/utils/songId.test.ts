import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const inject = vi.fn();
// Keyed the way the chat store keys its slices: lowercased.
const rooms = new Map<string, unknown>();
vi.mock('../stores/chatConnectionStore', () => ({
  injectSystemMessage: (...args: unknown[]) => inject(...args),
  systemSourceFor: () => undefined,
  useChatConnectionStore: { getState: () => ({ channels: rooms }) },
}));
// The room the main view holds for the playing stream (AppStore mainChatRoom).
const main: { room: string | null } = { room: null };
vi.mock('../stores/AppStore', () => ({
  useAppStore: { getState: () => ({ settings: {}, currentStream: null }) },
  mainChatRoom: () => main.room,
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
// The answer over a chat's composer.
const publish = vi.fn();
vi.mock('./songResult', () => ({ publishSongResult: (...args: unknown[]) => publish(...args) }));

import { announceSong, type RecognizeResult } from './songId';

const match: RecognizeResult = {
  status: 'match',
  song: { title: 'One', artist: 'A', album: null, album_art: null, shazam_url: null, song_link: null, providers: [], key: 'k' },
};

describe('where a song answer is posted', () => {
  const dispatchEvent = vi.fn();
  beforeEach(() => {
    inject.mockClear();
    publish.mockClear();
    dispatchEvent.mockClear();
    rooms.clear();
    main.room = null;
    vi.stubGlobal('window', { dispatchEvent });
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('posts in the named room whatever case its key comes in', () => {
    rooms.set('xqc', {});
    rooms.set('youtube:ucabc', {});
    announceSong(match, 'XQC');
    announceSong(match, 'youtube:UCAbc');
    expect(inject.mock.calls.map((c) => c[0])).toEqual(['XQC', 'youtube:UCAbc']);
    expect(inject.mock.calls[0][2]).toEqual(match.status === 'match' ? match.song : null);
  });

  it('falls back to the open chat when no room is named (a stream with no chat of its own)', () => {
    announceSong({ status: 'no-match' }, null);
    expect(inject).not.toHaveBeenCalled();
    expect(dispatchEvent).toHaveBeenCalledTimes(1);
  });

  it("keeps the fallback for the main view's room held elsewhere (its chat in a popout)", () => {
    main.room = 'youtube:UCAbc';
    announceSong(match, 'YOUTUBE:ucabc');
    expect(inject).not.toHaveBeenCalled();
    expect(dispatchEvent).toHaveBeenCalledTimes(1);
  });

  it('posts nothing for a room this page no longer holds and the main view left', () => {
    main.room = 'xqc';
    announceSong(match, 'kick:gone');
    expect(inject).not.toHaveBeenCalled();
    expect(dispatchEvent).not.toHaveBeenCalled();
    // Nor does its answer show over any composer.
    expect(publish).not.toHaveBeenCalled();
  });

  it('shows the answer over the composer only where it was posted', () => {
    rooms.set('xqc', {});
    announceSong(match, 'XQC');
    expect(publish).toHaveBeenCalledWith(match, 'XQC');
    main.room = 'youtube:UCAbc';
    announceSong({ status: 'no-match' }, 'YOUTUBE:ucabc');
    expect(publish).toHaveBeenLastCalledWith({ status: 'no-match' }, 'YOUTUBE:ucabc');
    expect(publish).toHaveBeenCalledTimes(2);
  });

  it('says nothing for a canceled listening or one refused while another ran', () => {
    rooms.set('xqc', {});
    announceSong({ status: 'canceled' }, 'xqc');
    announceSong({ status: 'busy' }, 'xqc');
    announceSong({ status: 'busy' }, null);
    expect(inject).not.toHaveBeenCalled();
    expect(dispatchEvent).not.toHaveBeenCalled();
  });
});
