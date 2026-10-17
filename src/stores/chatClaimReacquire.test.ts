import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// Rust keeps a Twitch channel JOINed while any window has claimed it, and a
// window claims with start_chat / join_chat_channel and lets go with
// leave_chat_channel. These drive the real store with the Tauri calls and the
// socket faked, replay the window's calls the way Rust applies them, and check
// that a channel the window still holds is still claimed at the end.
//
// The case that broke: the window's first channel is released and taken back
// while its first connect is still in flight (a remount does exactly this).
// The release reached Rust after the connect's claim and PARTed the channel,
// and the re-acquire rode the in-flight connect without claiming again, so the
// window held a channel nobody had JOINed and showed no live rows.

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

/**
 * Whether Rust ends up holding `channel` for this window, replaying the
 * window's calls in the order it made them. Rust's claims are a set per
 * window, so a repeated claim is a no-op and one release drops it.
 */
function rustHolds(channel: string): boolean {
  let held = false;
  for (const [command, args] of invokeMock.mock.calls as [string, Record<string, unknown> | undefined][]) {
    if (args?.channel !== channel) continue;
    if (command === 'start_chat' && args.claim !== false) held = true;
    if (command === 'join_chat_channel') held = true;
    if (command === 'leave_chat_channel') held = false;
  }
  return held;
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal('WebSocket', FakeSocket);
  FakeSocket.opened = [];
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (command: string) => {
    if (command === 'start_chat' || command === 'chat_bridge_port') return 4242;
    if (command === 'load_channel_history') return [];
    return undefined;
  });
});

afterEach(async () => {
  const { useChatConnectionStore, releaseChannel } = await import('./chatConnectionStore');
  for (const slice of Array.from(useChatConnectionStore.getState().channels.values())) {
    for (let i = 0; i < slice.refCount; i += 1) {
      await releaseChannel(slice.channel, slice.provider);
    }
  }
  await vi.advanceTimersByTimeAsync(20_000);
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

async function settle() {
  for (let i = 0; i < 10; i += 1) await vi.advanceTimersByTimeAsync(10);
}

describe("a window's Twitch claim survives a release and re-acquire", () => {
  it('keeps the first channel claimed when a remount releases and re-takes it mid-connect', async () => {
    const { acquireChannel, releaseChannel, useChatConnectionStore } = await import('./chatConnectionStore');

    // Mount, cleanup, mount again, with nothing awaited in between.
    const mount = acquireChannel('caseoh_', '123', 'twitch');
    const cleanup = releaseChannel('caseoh_', 'twitch');
    const remount = acquireChannel('caseoh_', '123', 'twitch');
    await settle();
    await Promise.all([mount, cleanup, remount]);

    expect(useChatConnectionStore.getState().channels.get('caseoh_')?.refCount).toBe(1);
    expect(rustHolds('caseoh_')).toBe(true);
    // Still one socket for the window.
    expect(FakeSocket.opened).toHaveLength(1);
  });

  it('claims a different channel taken while the released one was still connecting', async () => {
    const { acquireChannel, releaseChannel, useChatConnectionStore } = await import('./chatConnectionStore');

    const first = acquireChannel('xqc', '71092938', 'twitch');
    const letGo = releaseChannel('xqc', 'twitch');
    const next = acquireChannel('shroud', '37402112', 'twitch');
    await settle();
    await Promise.all([first, letGo, next]);

    expect(Array.from(useChatConnectionStore.getState().channels.keys())).toEqual(['shroud']);
    expect(rustHolds('shroud')).toBe(true);
    expect(rustHolds('xqc')).toBe(false);
  });

  it('keeps every MultiChat tab claimed through a remount', async () => {
    const { acquireChannel, releaseChannel } = await import('./chatConnectionStore');

    const bg = { background: true };
    const mounts = [acquireChannel('xqc', '1', 'twitch', bg), acquireChannel('shroud', '2', 'twitch', bg)];
    const cleanups = [releaseChannel('xqc', 'twitch', bg), releaseChannel('shroud', 'twitch', bg)];
    const remounts = [acquireChannel('xqc', '1', 'twitch', bg), acquireChannel('shroud', '2', 'twitch', bg)];
    await settle();
    await Promise.all([...mounts, ...cleanups, ...remounts]);

    expect(rustHolds('xqc')).toBe(true);
    expect(rustHolds('shroud')).toBe(true);
  });

  it('a channel released for good stays released', async () => {
    const { acquireChannel, releaseChannel, useChatConnectionStore } = await import('./chatConnectionStore');

    await Promise.all([acquireChannel('caseoh_', '123', 'twitch'), settle()]);
    await releaseChannel('caseoh_', 'twitch');
    await settle();

    expect(useChatConnectionStore.getState().channels.size).toBe(0);
    expect(rustHolds('caseoh_')).toBe(false);
  });
});

describe('recent history when a channel is opened before its id is known', () => {
  it('loads history once the id arrives on a later open of the same channel', async () => {
    const { acquireChannel } = await import('./chatConnectionStore');
    const historyCalls = () => invokeMock.mock.calls.filter(([c]) => c === 'load_channel_history');

    // The chat dock (or a blended companion) opens it first, id unknown.
    const first = acquireChannel('lynka', null, 'twitch', { background: true });
    await settle();
    await first;
    expect(historyCalls()).toHaveLength(0);

    // The main chat opens the same channel with its id.
    const main = acquireChannel('lynka', '42', 'twitch');
    await settle();
    await main;
    expect(historyCalls()).toHaveLength(1);
    expect(historyCalls()[0][1]).toMatchObject({ channel: 'lynka' });

    // A third open changes nothing: the id is already known.
    const again = acquireChannel('lynka', '42', 'twitch');
    await settle();
    await again;
    expect(historyCalls()).toHaveLength(1);
  });
});
