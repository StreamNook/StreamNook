import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

// While the app boots, a link that arrived is still waiting in Rust. A start the
// person makes drops it (link_superseded); a start the app makes by itself, and
// Home's own tab choice and the restore of the saved platform, must not.

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(),
}));
vi.mock('../bootPreload', () => ({ takePreloadedSettings: () => null }));

const win = vi.hoisted(() => ({ main: true }));
vi.mock('../utils/platform', async (original) => ({
  ...(await original<typeof import('../utils/platform')>()),
  IS_MAIN_WINDOW: true,
  isMainWindow: () => win.main,
}));

import { useAppStore } from './AppStore';
import { noteLinkStamp, resetNoticeReportForTest, runAsAutomatic, watchBootActions } from '../utils/noticeReport';

const get = () => useAppStore.getState();
const stream = { user_login: 'zerator', user_name: 'ZeratoR', user_id: '41719107' };
const superseded = () => invokeMock.mock.calls.filter(([c]) => c === 'link_superseded');

/** Rust answers every start the way a healthy one does; the rest answers nothing. */
function answerEverything() {
  let n = 0;
  invokeMock.mockImplementation(async (cmd: string) => {
    if (cmd === 'start_stream') return { url: `http://localhost/relay-${++n}.m3u8`, quality: 'best', ticket: n };
    if (cmd === 'watch_start') {
      return { superseded: false, row: stream, stream: { url: `http://localhost/relay-${++n}.m3u8`, quality: 'best', ticket: n } };
    }
    if (cmd === 'watch_open_room') return { superseded: false, room: { login: 'zerator', latest_vod: null } };
    return undefined;
  });
}

beforeEach(() => {
  vi.useFakeTimers();
  invokeMock.mockReset();
  answerEverything();
  resetNoticeReportForTest();
  win.main = true;
  noteLinkStamp(7);
  useAppStore.setState({
    isBooting: true,
    currentStream: null,
    streamUrl: null,
    currentMediaType: null,
    settings: { quality: 'best' } as never,
    isAuthenticated: false,
    addToast: vi.fn() as never,
    offlineRoom: { latest_vod: { id: '9' } } as never,
  });
});

afterEach(() => vi.useRealTimers());

/** Runs `start` (plainly, or under the automatic flag) and lets everything it awaits finish. */
async function run(start: () => Promise<unknown>, automatically = false) {
  let p: Promise<unknown> = Promise.resolve();
  if (automatically) runAsAutomatic(() => void (p = start()));
  else p = start();
  // Not awaited to its end: a provider start goes on to open a chat socket, and
  // the entry lines under test have all run long before that.
  p.catch(() => {});
  await vi.advanceTimersByTimeAsync(3000);
}

// The count is how many times the start's own lines run: a provider channel
// goes through startStream and then startProviderStream, and each notes it.
const starts: Array<[string, () => Promise<unknown>, number]> = [
  ['startStream', () => get().startStream('zerator', stream as never), 1],
  ['startOfflineChat', () => get().startOfflineChat('zerator', stream as never), 1],
  ['startProviderStream (through startStream)', () => get().startStream('kick:zerator', { ...stream, provider: 'kick' } as never), 2],
  ['playMedia', () => get().playMedia('video', 'https://www.twitch.tv/videos/1', { id: '1', title: 'A video' } as never), 1],
  ['playOfflineRoomVod', () => { useAppStore.setState({ currentMediaType: 'offline_chat' }); return get().playOfflineRoomVod(); }, 1],
];

describe('a start the person makes while the app boots', () => {
  it.each(starts)('%s tells Rust the page acted, with the stamp the page has heard', async (_name, start, count) => {
    // Fails against the old store: no start has the entry line.
    await run(start);
    expect(superseded()).toHaveLength(count);
    expect(superseded()[0][1]).toEqual({ seen: 7 });
  });

  it.each(starts)('%s tells Rust nothing once boot is over', async (_name, start) => {
    useAppStore.setState({ isBooting: false });
    await run(start);
    expect(superseded()).toHaveLength(0);
  });

  it.each(starts)('%s tells Rust nothing from a popout', async (_name, start) => {
    // A popout's store never leaves isBooting, so only the window check keeps it quiet.
    win.main = false;
    await run(start);
    expect(superseded()).toHaveLength(0);
  });

  it('playOfflineRoomVod with nothing to play opened nothing, so it tells Rust nothing', async () => {
    useAppStore.setState({ currentMediaType: 'offline_chat', offlineRoom: { latest_vod: null } as never });
    await run(() => get().playOfflineRoomVod());
    expect(superseded()).toHaveLength(0);
  });
});

describe('a start the app makes by itself while the app boots', () => {
  it.each(starts)('%s tells Rust nothing under runAsAutomatic, hand-overs included', async (_name, start) => {
    // The flag lasts only through the call's synchronous part, so a start that
    // hands over after an await (the provider path) has to carry it. Fails
    // without keepAutomatic: the provider case then makes one call.
    await run(start, true);
    expect(superseded()).toHaveLength(0);
  });

  it('a plain start right after an automatic one still tells Rust (the flag does not stick)', async () => {
    await run(starts[0][1], true);
    expect(superseded()).toHaveLength(0);
    await run(starts[3][1]);
    expect(superseded()).toHaveLength(1);
  });
});

describe('autoSelectHomeTab', () => {
  it('sets the tab', () => {
    useAppStore.setState({ homeActiveTab: 'following' });
    get().autoSelectHomeTab('recommended');
    expect(get().homeActiveTab).toBe('recommended');
  });

  it('is not a page the boot watch counts, while a tab click is', async () => {
    // Fails if autoSelectHomeTab is a plain set: Home's own choice during boot
    // would drop a link no person overrode.
    const stop = watchBootActions(useAppStore, ((cmd: string, args?: Record<string, unknown>) => invokeMock(cmd, args)) as never);
    useAppStore.setState({ homeActiveTab: 'following' });
    invokeMock.mockClear();
    get().autoSelectHomeTab('recommended');
    await vi.advanceTimersByTimeAsync(10);
    expect(superseded()).toHaveLength(0);
    get().setHomeActiveTab('browse');
    await vi.advanceTimersByTimeAsync(10);
    expect(superseded()).toHaveLength(1);
    stop();
  });
});

describe('loadSettings restores the saved platform', () => {
  it('as an automatic write the boot watch ignores', async () => {
    // Fails if the restore is a plain set: a person's platform choice and the
    // app's restore of the saved one would look the same to the watch.
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === 'load_settings' ? { active_platform: 'twitch', chat_placement: 'right' } : undefined,
    );
    useAppStore.setState({ activePlatform: 'all' });
    const stop = watchBootActions(useAppStore, ((cmd: string, args?: Record<string, unknown>) => invokeMock(cmd, args)) as never);
    await get().loadSettings();
    await vi.advanceTimersByTimeAsync(10);
    expect(get().activePlatform).toBe('twitch');
    expect(superseded()).toHaveLength(0);
    // A person's choice of platform is an action.
    get().setActivePlatform('all');
    await vi.advanceTimersByTimeAsync(10);
    expect(superseded()).toHaveLength(1);
    stop();
  });
});

describe('the entry lines (source pins)', () => {
  const text = readFileSync(resolve(__dirname, 'AppStore.ts'), 'utf8').replace(/\r\n/g, '\n');
  const NOTE = 'if (get().isBooting && isMainWindow()) noteBootAction(invoke);';

  /** The text of a function from its header to the first await in it. */
  function entry(header: string): string {
    const from = text.lastIndexOf(header);
    expect(from).toBeGreaterThan(-1);
    const to = text.indexOf('await ', from);
    return text.slice(from, to);
  }

  it.each([
    ['async function startProviderStream('],
    ['\n  startStream: async ('],
    ['\n  startOfflineChat: async ('],
    ['\n  playMedia: async ('],
  ])('%s notes the boot action before its first await', (header) => {
    expect(entry(header)).toContain(NOTE);
  });

  it('playOfflineRoomVod notes it after its guard and before it changes anything', () => {
    const from = text.lastIndexOf('\n  playOfflineRoomVod: async () => {');
    const guard = text.indexOf("get().currentMediaType !== 'offline_chat') return;", from);
    const note = text.indexOf(NOTE, from);
    const write = text.indexOf('set({ isLoading: true });', from);
    expect(guard).toBeGreaterThan(from);
    expect(note).toBeGreaterThan(guard);
    expect(note).toBeLessThan(write);
  });

  it('every start that hands over to another keeps the automatic flag', () => {
    expect(text).toContain('return keepAutomatic(automatic, () => startProviderStream(');
    expect(text).toContain('await keepAutomatic(automatic, () => get().startOfflineChat(');
    expect(text).toContain('await keepAutomatic(automatic, () => get().startStream(login, opened.live));');
    expect(text).toContain('await keepAutomatic(automatic, () => get().playOfflineRoomVod());');
  });
});
