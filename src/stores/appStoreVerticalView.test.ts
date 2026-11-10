import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// The phone's vertical view in the store (SN-40): every start carries the
// wanted view, a restart never falls back to the picture it happened to be
// served, the pill goes through change_stream_view only, a quality pick never
// saves a view, a turn restarts only a stream with both pictures, and a late
// v2 answer is applied only for the start it names.

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
const handlers = vi.hoisted(() => new Map<string, (e: { payload: unknown }) => void>());
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, fn: (e: { payload: unknown }) => void) => {
    handlers.set(name, fn);
    return () => {};
  }),
  emit: vi.fn(),
}));
vi.mock('../bootPreload', () => ({ takePreloadedSettings: () => null }));
vi.mock('../utils/platform', async (original) => ({
  ...(await original<typeof import('../utils/platform')>()),
  IS_MOBILE: true,
}));

import { useAppStore, type StreamStartResult } from './AppStore';

const stream = { user_login: 'zerator', user_name: 'ZeratoR', user_id: '41719107' };
const calls = (cmd: string) => invokeMock.mock.calls.filter(([c]) => c === cmd);
const lastArgs = (cmd: string) => calls(cmd).at(-1)?.[1] as Record<string, unknown> | undefined;

let starts = 0;
/** Rust answers every start; `answer` shapes the result. */
function answer(shape: (cmd: string, args: Record<string, unknown>) => Partial<StreamStartResult> = () => ({})) {
  invokeMock.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
    if (cmd === 'start_stream' || cmd === 'change_stream_quality' || cmd === 'change_stream_view' || cmd === 'rewind_live_stream') {
      starts += 1;
      return { url: `http://localhost/relay-${starts}.m3u8`, quality: 'best', ticket: starts, ...shape(cmd, args) };
    }
    if (cmd === 'watch_start') {
      starts += 1;
      return {
        superseded: false,
        row: stream,
        stream: { url: `http://localhost/relay-${starts}.m3u8`, quality: 'best', ticket: starts, ...shape(cmd, args) },
      };
    }
    if (cmd === 'check_stream_online') return stream;
    return undefined;
  });
}

beforeEach(() => {
  vi.useFakeTimers();
  invokeMock.mockReset();
  starts = 0;
  answer();
  useAppStore.setState({
    currentStream: stream as never,
    currentMediaType: 'live',
    settings: { quality: 'best' } as never,
    streamUrl: 'http://localhost/old.m3u8',
    isAutoSwitching: false,
    isRestartingStream: false,
    isAuthenticated: false,
    addToast: vi.fn() as never,
    streamView: 'landscape',
    verticalAvailable: true,
    streamPortrait: false,
    streamViewPick: null,
    changingStreamView: false,
    verticalAdsNote: false,
    streamPosture: 'upright',
    streamTvOwns: false,
    liveRewind: null,
    vodPlayback: null,
  });
});

afterEach(() => vi.useRealTimers());

async function settle<T>(p: Promise<T>): Promise<T> {
  await vi.advanceTimersByTimeAsync(2000);
  return p;
}

describe('restarts carry the wanted view', () => {
  it('sends the session pick, not the picture served', async () => {
    useAppStore.setState({ streamViewPick: 'vertical', streamView: 'landscape' });
    await settle(useAppStore.getState().restartStream({ quiet: true }));
    expect(lastArgs('start_stream')).toMatchObject({ url: 'https://twitch.tv/zerator', quality: 'best', view: 'vertical' });
  });

  it('sends landscape while the phone is turned, whatever was picked', async () => {
    useAppStore.setState({ streamViewPick: 'vertical', streamPosture: 'landscape' });
    await settle(useAppStore.getState().restartStream({ quiet: true }));
    expect(lastArgs('start_stream')?.view).toBe('landscape');
  });

  it('sends landscape while the TV owns the picture', async () => {
    useAppStore.setState({ streamViewPick: 'vertical', streamTvOwns: true });
    await settle(useAppStore.getState().restartStream({ quiet: true }));
    expect(lastArgs('start_stream')?.view).toBe('landscape');
  });

  it('sends a null view (the remembered pick) when nothing was picked', async () => {
    await settle(useAppStore.getState().restartStream({ quiet: true }));
    const args = lastArgs('start_stream');
    expect(args && 'view' in args).toBe(true);
    expect(args?.view).toBeNull();
  });

  it('takes the served picture, the second picture and the shape from the answer', async () => {
    answer(() => ({ view: 'vertical', vertical_available: true, portrait: true }));
    useAppStore.setState({ streamViewPick: 'vertical' });
    await settle(useAppStore.getState().restartStream({ quiet: true }));
    const s = useAppStore.getState();
    expect(s.streamView).toBe('vertical');
    expect(s.verticalAvailable).toBe(true);
    expect(s.streamPortrait).toBe(true);
  });

  it('rewinds and returns to live in the wanted view', async () => {
    useAppStore.setState({ streamViewPick: 'vertical', liveRewindAvailable: true });
    await settle(useAppStore.getState().rewindLive({ behindSecs: 60 }));
    expect(lastArgs('rewind_live_stream')?.view).toBe('vertical');
    useAppStore.setState({ liveRewind: { channel: 'zerator', videoId: '1' } });
    await settle(useAppStore.getState().returnToLive());
    expect(lastArgs('start_stream')?.view).toBe('vertical');
  });

  it('swaps to audio and back in the picture it serves, never another', async () => {
    useAppStore.setState({ streamView: 'vertical', streamViewPick: null });
    await settle(useAppStore.getState().applyTransientQuality('audio_only'));
    expect(lastArgs('start_stream')).toMatchObject({ quality: 'audio_only', view: 'vertical' });
  });
});

describe('the quality pick', () => {
  it('keeps the wanted view and never saves one', async () => {
    useAppStore.setState({ streamViewPick: 'vertical' });
    await settle(useAppStore.getState().changeStreamQuality('720p60'));
    expect(lastArgs('change_stream_quality')).toMatchObject({ quality: '720p60', view: 'vertical' });
    expect(calls('change_stream_view')).toHaveLength(0);
    const wrote = invokeMock.mock.calls.some(([, a]) => JSON.stringify(a ?? {}).includes('vertical_view'));
    expect(wrote).toBe(false);
  });
});

describe('the pill', () => {
  it('reloads through change_stream_view and takes its answer', async () => {
    answer(() => ({
      view: 'vertical',
      vertical_available: true,
      portrait: true,
      gave_up_ad_free: true,
      mode: 'auth-only',
      reason: 'vertical',
    }));
    await settle(useAppStore.getState().changeStreamView('vertical'));
    // The Ad-free page says why no relay plays.
    expect(useAppStore.getState().adSource).toEqual({ mode: 'auth-only', entitled: false, region: undefined, reason: 'vertical' });
    expect(lastArgs('change_stream_view')).toEqual({ url: 'https://twitch.tv/zerator', view: 'vertical', quality: null });
    expect(calls('change_stream_quality')).toHaveLength(0);
    const s = useAppStore.getState();
    expect(s.streamView).toBe('vertical');
    expect(s.streamPortrait).toBe(true);
    expect(s.streamViewPick).toBe('vertical');
    expect(s.streamUrl).toBe('http://localhost/relay-1.m3u8');
    expect(s.changingStreamView).toBe(false);
    // The note by the pill, on its own clock; no quality toast.
    expect(s.verticalAdsNote).toBe(true);
    expect(s.addToast).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(6000);
    expect(useAppStore.getState().verticalAdsNote).toBe(false);
  });

  it('puts the pick back when the switch fails', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'change_stream_view') throw new Error('usher down');
      return undefined;
    });
    await settle(useAppStore.getState().changeStreamView('vertical'));
    const s = useAppStore.getState();
    expect(s.streamViewPick).toBeNull();
    expect(s.streamUrl).toBe('http://localhost/old.m3u8');
    expect(s.isRestartingStream).toBe(false);
    expect(s.changingStreamView).toBe(false);
  });

  it('keeps a VOD at its playhead and sends its own URL', async () => {
    answer(() => ({ view: 'vertical', vod: { video_id: '9', start_position_secs: 0 } as never }));
    useAppStore.setState({
      currentMediaType: 'video',
      originalMediaUrl: 'https://www.twitch.tv/videos/9',
      vodPlayback: { video_id: '9', start_position_secs: 0 } as never,
    });
    await settle(useAppStore.getState().changeStreamView('vertical'));
    expect(lastArgs('change_stream_view')).toEqual({ url: 'https://www.twitch.tv/videos/9', view: 'vertical', quality: null });
    expect(useAppStore.getState().vodPlayback?.video_id).toBe('9');
  });
});

describe('a turn', () => {
  it('restarts a stream with both pictures into the one now wanted, quietly', async () => {
    useAppStore.setState({ streamView: 'vertical', streamViewPick: 'vertical', verticalAvailable: true });
    useAppStore.getState().setStreamPosture('landscape', false);
    await vi.advanceTimersByTimeAsync(3000);
    expect(lastArgs('start_stream')?.view).toBe('landscape');
    expect(useAppStore.getState().addToast).not.toHaveBeenCalled();
  });

  it('never restarts a stream with one picture', async () => {
    useAppStore.setState({ streamView: 'landscape', verticalAvailable: false });
    useAppStore.getState().setStreamPosture('landscape', false);
    await vi.advanceTimersByTimeAsync(3000);
    expect(calls('start_stream')).toHaveLength(0);
  });

  it('never restarts a dual stream already playing what the turn wants', async () => {
    useAppStore.setState({ streamView: 'landscape', verticalAvailable: true, streamViewPick: null });
    useAppStore.getState().setStreamPosture('landscape', false);
    await vi.advanceTimersByTimeAsync(3000);
    expect(calls('start_stream')).toHaveLength(0);
  });
});

describe('a late v2 answer', () => {
  async function open(shape: Partial<StreamStartResult>) {
    answer((cmd) => (cmd === 'watch_start' ? shape : {}));
    useAppStore.setState({ currentStream: null, streamUrl: null, currentMediaType: null });
    await settle(useAppStore.getState().startStream('zerator', stream as never));
  }

  it('shows the pill only for the start it names', async () => {
    await open({ view: 'landscape', wanted: 'landscape', vertical_available: false });
    const late = handlers.get('vertical-available');
    expect(late).toBeDefined();
    // The open was this test's first start.
    const ticket = 1;
    expect(useAppStore.getState().streamUrl).toBe('http://localhost/relay-1.m3u8');
    late!({ payload: { ticket: ticket + 5, available: true } });
    // Ahead of its start's result: held, not shown.
    expect(useAppStore.getState().verticalAvailable).toBe(false);
    late!({ payload: { ticket: 0, available: true } });
    expect(useAppStore.getState().verticalAvailable).toBe(false);
    late!({ payload: { ticket, available: true } });
    expect(useAppStore.getState().verticalAvailable).toBe(true);
    // Remembered landscape: nothing restarts.
    await vi.advanceTimersByTimeAsync(3000);
    expect(calls('start_stream')).toHaveLength(0);
  });

  it('restarts into a remembered vertical once the portrait picture is found', async () => {
    await open({ view: 'landscape', wanted: 'vertical', vertical_available: false });
    handlers.get('vertical-available')!({ payload: { ticket: 1, available: true } });
    await vi.advanceTimersByTimeAsync(3000);
    // The remembered pick: the start sends no view of its own.
    expect(calls('start_stream')).toHaveLength(1);
    expect(lastArgs('start_stream')?.view).toBeNull();
  });
});

describe('what played is gone', () => {
  it('starts the view over on another channel, keeps it on the same one', async () => {
    useAppStore.setState({ streamViewPick: 'vertical', streamView: 'vertical' });
    answer(() => ({ view: 'vertical' }));
    await settle(useAppStore.getState().startStream('zerator', stream as never));
    expect(useAppStore.getState().streamViewPick).toBe('vertical');
    await settle(useAppStore.getState().startStream('lultarek', { ...stream, user_login: 'lultarek' } as never));
    expect(useAppStore.getState().streamViewPick).toBeNull();
    expect(lastArgs('watch_start')?.view).toBeNull();
  });
});
