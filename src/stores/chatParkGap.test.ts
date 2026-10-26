import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// A chat tab that is not shown holds its frames in Rust. When more arrived than
// Rust kept, the window used to empty the tab; it now keeps the rows it showed
// and fills the span between them and the backlog from recent history.

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
  inlineEmoteTier: vi.fn(() => '2x'),
}));
vi.mock('../services/twitchBadges', () => ({
  parseBadges: vi.fn(() => []),
  initializeBadgeCache: vi.fn(async () => {}),
}));
vi.mock('../services/badgeImageCacheService', () => ({
  initializeBadgeImageCache: vi.fn(async () => {}),
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

const T0 = 1_700_000_000_000;

function row(id: string, at: number) {
  return {
    id,
    channel: 'parkroom',
    provider: 'twitch',
    timestamp: at,
    user_id: `u-${id}`,
    username: id,
    display_name: id,
    content: `message ${id}`,
    badges: [],
    tags: {},
    metadata: {},
  };
}

let backfillRows: ReturnType<typeof row>[] = [];

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(T0 + 60_000);
  vi.stubGlobal('WebSocket', FakeSocket);
  FakeSocket.opened = [];
  backfillRows = [];
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (command: string, args?: { afterMs?: number | null }) => {
    if (command === 'start_chat' || command === 'chat_bridge_port') return 4242;
    if (command === 'load_channel_history') return args?.afterMs ? backfillRows : [];
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
  // Past the join hold (1.5 s) and the flush latency (250 ms).
  for (let i = 0; i < 10; i += 1) await vi.advanceTimersByTimeAsync(250);
}

function ids(messages: unknown[]): string[] {
  return messages.map((m) => (typeof m === 'string' ? m : (m as { id: string }).id));
}

describe('a held-back chat that outran Rust\'s ring', () => {
  it('keeps its rows and fills the span from history without duplicates', async () => {
    const { acquireChannel, useChatConnectionStore } = await import('./chatConnectionStore');
    const opening = acquireChannel('parkroom', '42', 'twitch');
    await settle();
    await opening;
    const socket = FakeSocket.opened.at(-1);
    if (!socket) throw new Error('no socket opened');

    socket.frame(JSON.stringify(row('A', T0 + 1_000)));
    socket.frame(JSON.stringify(row('B', T0 + 2_000)));
    await settle();
    expect(ids(useChatConnectionStore.getState().channels.get('parkroom')?.messages ?? [])).toEqual(['A', 'B']);

    // Rust kept only E and F of what arrived while the tab was held back.
    backfillRows = [row('C', T0 + 3_000), row('D', T0 + 4_000), row('E', T0 + 5_000)];
    socket.frame('PARK_GAP:parkroom');
    await settle();
    expect(ids(useChatConnectionStore.getState().channels.get('parkroom')?.messages ?? [])).toEqual(['A', 'B']);

    socket.frame(JSON.stringify(row('E', T0 + 5_000)));
    socket.frame(JSON.stringify(row('F', T0 + 6_000)));
    socket.frame('PARK_END:parkroom');
    await settle();

    expect(ids(useChatConnectionStore.getState().channels.get('parkroom')?.messages ?? [])).toEqual([
      'A',
      'B',
      'C',
      'D',
      'E',
      'F',
    ]);
    const backfills = invokeMock.mock.calls.filter(
      ([command, args]) => command === 'load_channel_history' && (args as { afterMs?: number }).afterMs,
    );
    expect(backfills).toHaveLength(1);
    expect((backfills[0][1] as { afterMs: number }).afterMs).toBe(T0 + 2_000 - 30_000);
  });

  it('asks for no history when nothing was lost', async () => {
    const { acquireChannel } = await import('./chatConnectionStore');
    const opening = acquireChannel('parkroom', '42', 'twitch');
    await settle();
    await opening;
    const socket = FakeSocket.opened.at(-1);
    if (!socket) throw new Error('no socket opened');

    socket.frame(JSON.stringify(row('A', T0 + 1_000)));
    socket.frame(JSON.stringify(row('B', T0 + 2_000)));
    socket.frame('PARK_END:parkroom');
    await settle();

    const backfills = invokeMock.mock.calls.filter(
      ([command, args]) => command === 'load_channel_history' && (args as { afterMs?: number }).afterMs,
    );
    expect(backfills).toHaveLength(0);
  });
});
