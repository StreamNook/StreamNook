import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// Each channel in a chat window connects on its own. These drive the real store
// with the Tauri calls and the socket faked, and check that one platform's
// failed connect never costs the window its socket or the other channels.

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}), emit: vi.fn() }));
vi.mock('./AppStore', () => ({
  useAppStore: {
    getState: () => ({ settings: {}, currentUser: null }),
    setState: vi.fn(),
    subscribe: () => () => {},
  },
}));
vi.mock('../services/emoteService', () => ({
  fetchAllEmotes: vi.fn(async () => null),
  fetchKickChannelEmotes: vi.fn(async () => null),
  fetchYouTubeChannelEmotes: vi.fn(async () => null),
  enhanceRustEmotes: vi.fn((x: unknown) => x),
}));
vi.mock('../services/twitchBadges', () => ({
  parseBadges: vi.fn(() => []),
  initializeBadgeCache: vi.fn(async () => {}),
}));
vi.mock('../utils/platform', () => ({ IS_MOBILE: false }));

class FakeSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  static opened: FakeSocket[] = [];
  readyState = FakeSocket.CONNECTING;
  onopen: ((e: unknown) => void) | null = null;
  onmessage: ((e: unknown) => void) | null = null;
  onerror: ((e: unknown) => void) | null = null;
  onclose: ((e: unknown) => void) | null = null;
  constructor(public url: string) {
    setTimeout(() => {
      this.readyState = FakeSocket.OPEN;
      FakeSocket.opened.push(this);
      this.onopen?.({});
    }, 0);
  }
  send() {}
  close() {
    this.readyState = FakeSocket.CLOSED;
  }
}

type Answer = (args: Record<string, unknown> | undefined) => unknown;
let answers: Record<string, Answer> = {};

function calls(command: string) {
  return invokeMock.mock.calls.filter((c) => c[0] === command);
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal('WebSocket', FakeSocket);
  FakeSocket.opened = [];
  // Kick panes seed from scrollback on open.
  answers = { kick_chat_history: () => [] };
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
    const answer = answers[command];
    return answer ? answer(args) : undefined;
  });
});

afterEach(async () => {
  const { useChatConnectionStore, releaseChannel } = await import('./chatConnectionStore');
  for (const slice of Array.from(useChatConnectionStore.getState().channels.values())) {
    for (let i = 0; i < slice.refCount; i += 1) {
      await releaseChannel(slice.channel.includes(':') ? slice.channel.split(':')[1] : slice.channel, slice.provider);
    }
  }
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

/** Await `p` while the fake clock runs, since the socket opens on a timer. */
async function withClock<T>(p: Promise<T>): Promise<T> {
  await settle();
  return p;
}

async function settle() {
  // Let the connect's awaits and the fake socket's open run.
  for (let i = 0; i < 10; i += 1) await vi.advanceTimersByTimeAsync(10);
}

describe('each chat channel connects on its own', () => {
  it('an offline YouTube channel opened first still gives the window its socket', async () => {
    answers.chat_bridge_port = () => 4242;
    answers.provider_chat_connect = (a) =>
      a?.provider === 'youtube' ? Promise.reject(new Error("'x' isn't live right now")) : Promise.resolve(4242);
    answers.start_chat = () => 4242;
    answers.join_chat_channel = () => undefined;
    const { acquireChannel, useChatConnectionStore } = await import('./chatConnectionStore');

    await withClock(acquireChannel('UCoffline', null, 'youtube'));
    expect(FakeSocket.opened.map((s) => s.url)).toEqual(['ws://localhost:4242']);

    await withClock(acquireChannel('xqc', '71092938', 'twitch'));
    const channels = useChatConnectionStore.getState().channels;
    expect(channels.get('xqc')?.isConnected).toBe(true);
    const youtube = Array.from(channels.values()).find((s) => s.provider === 'youtube');
    expect(youtube?.isConnected).toBe(false);
    expect(youtube?.error).toMatch(/isn't live/);
    // One socket for the window, whatever the YouTube channel did.
    expect(FakeSocket.opened).toHaveLength(1);
  });

  it('a Twitch start that fails still opens the socket for the others', async () => {
    answers.start_chat = () => Promise.reject(new Error('IRC connect failed'));
    answers.chat_bridge_port = () => 5151;
    answers.provider_chat_connect = () => 5151;
    const { acquireChannel, useChatConnectionStore } = await import('./chatConnectionStore');

    await withClock(acquireChannel('xqc', '71092938', 'twitch'));
    expect(FakeSocket.opened.map((s) => s.url)).toEqual(['ws://localhost:5151']);
    expect(useChatConnectionStore.getState().channels.get('xqc')?.error).toMatch(/IRC connect failed/);

    await withClock(acquireChannel('xqc', null, 'kick'));
    const kick = Array.from(useChatConnectionStore.getState().channels.values()).find((s) => s.provider === 'kick');
    expect(kick?.isConnected).toBe(true);
  });

  it('a failed channel retries alone, and stops once released', async () => {
    let tries = 0;
    answers.chat_bridge_port = () => 4242;
    answers.provider_chat_connect = () => {
      tries += 1;
      return tries < 3 ? Promise.reject(new Error('not live yet')) : Promise.resolve(4242);
    };
    const { acquireChannel, useChatConnectionStore } = await import('./chatConnectionStore');

    await withClock(acquireChannel('somecreator', null, 'tiktok'));
    expect(tries).toBe(1);

    await vi.advanceTimersByTimeAsync(30_000);
    await settle();
    expect(tries).toBe(2);

    // Backs off: the next try waits twice as long.
    await vi.advanceTimersByTimeAsync(30_000);
    await settle();
    expect(tries).toBe(2);
    await vi.advanceTimersByTimeAsync(30_000);
    await settle();
    expect(tries).toBe(3);

    const tiktok = Array.from(useChatConnectionStore.getState().channels.values()).find((s) => s.provider === 'tiktok');
    expect(tiktok?.isConnected).toBe(true);

    // Connected: nothing further is scheduled.
    await vi.advanceTimersByTimeAsync(600_000);
    expect(calls('provider_chat_connect')).toHaveLength(3);
  });

  it('a released channel is not retried', async () => {
    answers.chat_bridge_port = () => 4242;
    answers.provider_chat_connect = () => Promise.reject(new Error('not live'));
    const { acquireChannel, releaseChannel } = await import('./chatConnectionStore');

    await withClock(acquireChannel('keepsoffline', null, 'tiktok'));
    await withClock(acquireChannel('xqc', '71092938', 'twitch'));
    await releaseChannel('keepsoffline', 'tiktok');
    await vi.advanceTimersByTimeAsync(600_000);
    expect(calls('provider_chat_connect')).toHaveLength(1);
  });
});
