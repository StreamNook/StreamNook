import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// SN-270: changing the quality of a VOD (or a rewound broadcast) picks up where
// it was, on desktop and on the phone, instead of starting it again at 0:00.

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}), emit: vi.fn() }));
vi.mock('../bootPreload', () => ({ takePreloadedSettings: () => null }));
vi.mock('../utils/platform', async (original) => ({
  ...(await original<typeof import('../utils/platform')>()),
  IS_MOBILE: false,
}));

import { useAppStore } from './AppStore';
import { registerPlayerControls, registerPlayheadClock } from '../keybindings/playerControls';

const stream = { user_login: 'chan', user_name: 'Chan', user_id: '1' };
const vod = { video_id: '555', start_position_secs: 0 };
const argsOf = (cmd: string) => invokeMock.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a);

const desktopPlayerAt = (secs: number) =>
  registerPlayerControls({ getCurrentTime: () => secs } as never);

beforeEach(() => {
  vi.useFakeTimers();
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (cmd: string) =>
    cmd === 'change_stream_quality' ? { url: 'http://localhost/vod-720.m3u8', quality: '720p60', vod } : undefined,
  );
  useAppStore.setState({
    currentStream: stream as never,
    currentMediaType: 'video',
    originalMediaUrl: 'https://www.twitch.tv/videos/555',
    vodPlayback: vod as never,
    liveRewind: null,
    settings: { quality: 'best' } as never,
    streamUrl: 'http://localhost/vod-best.m3u8',
    isRestartingStream: false,
    addToast: vi.fn() as never,
  });
});

afterEach(() => {
  registerPlayerControls(null);
  vi.useRealTimers();
});

const change = async (quality: string) => {
  const done = useAppStore.getState().changeStreamQuality(quality);
  await vi.advanceTimersByTimeAsync(1000);
  await done;
};

describe('a quality change on a VOD', () => {
  it('starts the new quality where the desktop player was', async () => {
    desktopPlayerAt(1234.5);
    await change('720p60');
    expect(argsOf('change_stream_quality')).toEqual([{ url: 'https://www.twitch.tv/videos/555', quality: '720p60' }]);
    const s = useAppStore.getState();
    expect(s.streamUrl).toBe('http://localhost/vod-720.m3u8');
    expect(s.vodPlayback?.start_position_secs).toBe(1234.5);
  });

  it("reads the phone's player when it is the one mounted", async () => {
    desktopPlayerAt(5);
    const unregister = registerPlayheadClock(() => 812);
    await change('480p30');
    unregister();
    expect(useAppStore.getState().vodPlayback?.start_position_secs).toBe(812);
  });

  it('keeps the start it had when no player can say where it is', async () => {
    useAppStore.setState({ vodPlayback: { ...vod, start_position_secs: 60 } as never });
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === 'change_stream_quality' ? { url: 'http://localhost/vod-720.m3u8', quality: '720p60' } : undefined,
    );
    await change('720p60');
    expect(useAppStore.getState().vodPlayback?.start_position_secs).toBe(60);
  });

  it('a rewound broadcast stays in its recording at the playhead, not back at live', async () => {
    desktopPlayerAt(300);
    useAppStore.setState({ currentMediaType: 'live', originalMediaUrl: null, liveRewind: { channel: 'chan', videoId: '555' } });
    await change('720p60');
    expect(argsOf('change_stream_quality')).toEqual([{ url: 'https://www.twitch.tv/videos/555', quality: '720p60' }]);
    expect(useAppStore.getState().vodPlayback?.start_position_secs).toBe(300);
  });

  it('live is unchanged: the live url, and no VOD start is written', async () => {
    desktopPlayerAt(300);
    useAppStore.setState({ currentMediaType: 'live', originalMediaUrl: null, vodPlayback: null });
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === 'change_stream_quality' ? { url: 'http://localhost/live.m3u8', quality: '720p60' } : undefined,
    );
    await change('720p60');
    expect(argsOf('change_stream_quality')).toEqual([{ url: 'https://twitch.tv/chan', quality: '720p60' }]);
    expect(useAppStore.getState().vodPlayback).toBeNull();
  });
});
