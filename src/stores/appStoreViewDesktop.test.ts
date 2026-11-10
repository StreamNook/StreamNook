import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// Desktop never sends a view (SN-40): its starts carry the same arguments as
// before the phones' vertical view, and no vertical state is ever written.

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}), emit: vi.fn() }));
vi.mock('../bootPreload', () => ({ takePreloadedSettings: () => null }));
vi.mock('../utils/platform', async (original) => ({
  ...(await original<typeof import('../utils/platform')>()),
  IS_MOBILE: false,
}));

import { useAppStore } from './AppStore';

const stream = { user_login: 'chan', user_name: 'Chan', user_id: '1' };
const argsOf = (cmd: string) => invokeMock.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a);

beforeEach(() => {
  vi.useFakeTimers();
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (cmd: string) =>
    cmd === 'start_stream' || cmd === 'change_stream_quality'
      ? { url: 'http://localhost/relay.m3u8', quality: 'best', view: 'vertical', vertical_available: true, portrait: true, ticket: 3 }
      : undefined,
  );
  useAppStore.setState({
    currentStream: stream as never,
    currentMediaType: 'live',
    settings: { quality: 'best' } as never,
    streamUrl: 'http://localhost/old.m3u8',
    isAutoSwitching: false,
    isRestartingStream: false,
    addToast: vi.fn() as never,
  });
});

afterEach(() => vi.useRealTimers());

describe('desktop starts', () => {
  it('send exactly the url and quality, and write no vertical state', async () => {
    const restart = useAppStore.getState().restartStream({ quiet: true });
    await vi.advanceTimersByTimeAsync(1000);
    await restart;
    expect(argsOf('start_stream')).toEqual([{ url: 'https://twitch.tv/chan', quality: 'best' }]);
    const quality = useAppStore.getState().changeStreamQuality('720p60');
    await vi.advanceTimersByTimeAsync(1000);
    await quality;
    expect(argsOf('change_stream_quality')).toEqual([{ url: 'https://twitch.tv/chan', quality: '720p60' }]);
    const s = useAppStore.getState();
    expect(s.streamView).toBeNull();
    expect(s.verticalAvailable).toBe(false);
    expect(s.streamPortrait).toBeNull();
  });

  it('never runs the pill or a turn', async () => {
    await useAppStore.getState().changeStreamView('vertical');
    useAppStore.getState().setStreamPosture('landscape', false);
    await vi.advanceTimersByTimeAsync(2000);
    expect(argsOf('change_stream_view')).toHaveLength(0);
    expect(argsOf('start_stream')).toHaveLength(0);
    expect(useAppStore.getState().streamPosture).toBe('upright');
  });
});
