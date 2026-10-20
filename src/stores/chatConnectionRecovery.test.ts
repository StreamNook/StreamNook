import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// Two ways a phone chat went black and reloaded under the viewer:
//
// - The stale warning read chat frames alone, so a healthy room that stayed
//   quiet for two minutes (an offline channel) looked dead and was rebuilt.
//   Rust's heartbeat keeps arriving in a quiet room and stops when the
//   connection dies, so it is what the warning asks now.
// - The rebuild let go of every room and took it back, which emptied each pane
//   and painted it again from history. It now keeps the rooms and their rows
//   while Rust replaces the connection behind them.

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
vi.mock('../utils/platform', () => ({ IS_MOBILE: true }));

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
  /** A frame from Rust. */
  frame(data: string) {
    this.onmessage?.({ data });
  }
}

const STALE = 'Connection may be stale';

beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal('WebSocket', FakeSocket);
  FakeSocket.opened = [];
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (command: string) => {
    if (command === 'start_chat' || command === 'chat_bridge_port') return 4242;
    if (command === 'load_channel_history') return [];
    if (command === 'rebuild_chat') return 4343;
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

async function openRoom(channel: string) {
  const { acquireChannel } = await import('./chatConnectionStore');
  const opening = acquireChannel(channel, '42', 'twitch');
  await settle();
  await opening;
}

function liveSocket(): FakeSocket {
  const socket = FakeSocket.opened.at(-1);
  if (!socket) throw new Error('no socket opened');
  return socket;
}

describe('the stale warning', () => {
  it('counts the heartbeat as a sign of life', async () => {
    const { connectionSilentMs, staleWarningDue } = await import('./chatConnectionStore');
    const t0 = 1_000_000;
    // No frame for two and a half minutes, but a heartbeat thirty seconds ago.
    expect(connectionSilentMs(t0 + 150_000, t0, t0 + 120_000)).toBe(30_000);
    expect(staleWarningDue(t0 + 150_000, t0, t0 + 120_000)).toBe(false);
    // Neither for two and a half minutes: the connection is gone.
    expect(staleWarningDue(t0 + 150_000, t0, 0)).toBe(true);
    expect(staleWarningDue(t0 + 150_000, t0, t0 - 10_000)).toBe(true);
    // Not before two minutes, and the three-minute probe takes over after.
    expect(staleWarningDue(t0 + 119_000, t0, 0)).toBe(false);
    expect(staleWarningDue(t0 + 181_000, t0, 0)).toBe(false);
  });

  it('never marks a quiet room stale while heartbeats arrive', async () => {
    const { useChatConnectionStore } = await import('./chatConnectionStore');
    await openRoom('quietroom');
    const socket = liveSocket();
    const errors: (string | null)[] = [];
    for (let s = 0; s < 6 * 60; s += 30) {
      socket.frame('HEARTBEAT');
      await vi.advanceTimersByTimeAsync(30_000);
      errors.push(useChatConnectionStore.getState().channels.get('quietroom')?.error ?? null);
    }
    expect(errors.some((e) => typeof e === 'string' && e.startsWith(STALE))).toBe(false);
    // The room was never torn down either.
    expect(invokeMock.mock.calls.some(([c]) => c === 'leave_chat_channel' || c === 'rebuild_chat')).toBe(false);
  });

  it('marks the room stale once heartbeats stop too', async () => {
    const { useChatConnectionStore } = await import('./chatConnectionStore');
    await openRoom('deadroom');
    await vi.advanceTimersByTimeAsync(150_000);
    const error = useChatConnectionStore.getState().channels.get('deadroom')?.error ?? '';
    expect(error.startsWith(STALE)).toBe(true);
  });
});

describe('rebuilding the connection', () => {
  it('keeps every room and its rows while Rust replaces the connection', async () => {
    const { rebuildTwitchChat, injectSystemMessage, useChatConnectionStore } = await import('./chatConnectionStore');
    await openRoom('kathi');
    injectSystemMessage('kathi', 'a row on screen');
    await settle();
    const before = useChatConnectionStore.getState().channels.get('kathi');
    expect(before?.messages.length).toBeGreaterThan(0);
    const rowsBefore = before?.messages.slice() ?? [];
    const socketsBefore = FakeSocket.opened.length;

    const rebuilding = rebuildTwitchChat('no frames or heartbeats for two minutes');
    await settle();
    expect(await rebuilding).toBe(true);

    const after = useChatConnectionStore.getState().channels.get('kathi');
    expect(after).toBe(before);
    expect(after?.refCount).toBe(1);
    expect(after?.messages.slice(0, rowsBefore.length)).toEqual(rowsBefore);
    // Rust ran the cycle; the page never let go of the room to do it.
    expect(invokeMock).toHaveBeenCalledWith('rebuild_chat', {
      force: false,
      reason: 'no frames or heartbeats for two minutes',
    });
    expect(invokeMock.mock.calls.some(([c]) => c === 'leave_chat_channel' || c === 'stop_chat')).toBe(false);
    // And the window's socket attached to the rebuilt bridge.
    expect(FakeSocket.opened.length).toBe(socketsBefore + 1);
  });

  it('leaves the socket alone when Rust declines a cycle', async () => {
    const { rebuildTwitchChat } = await import('./chatConnectionStore');
    await openRoom('lirik');
    invokeMock.mockImplementation(async (command: string) => {
      if (command === 'start_chat' || command === 'chat_bridge_port') return 4242;
      if (command === 'load_channel_history') return [];
      return null;
    });
    const socketsBefore = FakeSocket.opened.length;
    const rebuilding = rebuildTwitchChat('away for 120s');
    await settle();
    expect(await rebuilding).toBe(false);
    expect(FakeSocket.opened.length).toBe(socketsBefore);
  });

  it('does nothing without a Twitch room', async () => {
    const { rebuildTwitchChat } = await import('./chatConnectionStore');
    expect(await rebuildTwitchChat('manual reload', true)).toBe(false);
    expect(invokeMock.mock.calls.some(([c]) => c === 'rebuild_chat')).toBe(false);
  });
});
