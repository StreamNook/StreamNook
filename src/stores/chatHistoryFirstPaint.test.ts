import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// A chat that opens commits its newest history rows first and the older ones a
// frame later, so the first paint is a handful of rows rather than the whole
// backlog mounting at once. A backfill after an outage stays a single step.

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}), emit: vi.fn() }));
vi.mock('./AppStore', () => ({
  useAppStore: {
    // A buffer cap above the 100 history rows plus the live ones, so nothing is
    // trimmed and the order of every row can be read off.
    getState: () => ({ settings: { chat_render: { message_buffer_cap: 200 } }, currentUser: null }),
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
    channel: 'openroom',
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

type Row = ReturnType<typeof row>;

/** `count` history rows, oldest first: H001, H002, ... */
function historyRows(count: number): Row[] {
  return Array.from({ length: count }, (_, i) => row(`H${String(i + 1).padStart(3, '0')}`, T0 + i * 100));
}

/** A promise the test settles by hand, standing in for Rust's answer. */
function gate<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

let historyGate = gate<Row[]>();
let backfillGate = gate<Row[]>();

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(T0 + 600_000);
  vi.stubGlobal('WebSocket', FakeSocket);
  FakeSocket.opened = [];
  historyGate = gate<Row[]>();
  backfillGate = gate<Row[]>();
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (command: string, args?: { afterMs?: number | null }) => {
    if (command === 'start_chat' || command === 'chat_bridge_port') return 4242;
    if (command === 'load_channel_history') return args?.afterMs ? backfillGate.promise : historyGate.promise;
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

function ids(messages: unknown[]): string[] {
  return messages.map((m) => (typeof m === 'string' ? m : (m as { id: string }).id));
}

const range = (from: number, to: number) =>
  Array.from({ length: to - from + 1 }, (_, i) => `H${String(from + i).padStart(3, '0')}`);

/** Opens the channel with two live rows already held behind the history. */
async function openWithLiveRows() {
  const { acquireChannel, useChatConnectionStore } = await import('./chatConnectionStore');
  const opening = acquireChannel('openroom', '42', 'twitch');
  await vi.advanceTimersByTimeAsync(50);
  await opening;
  const socket = FakeSocket.opened.at(-1);
  if (!socket) throw new Error('no socket opened');
  socket.frame(JSON.stringify(row('L1', T0 + 900_000)));
  socket.frame(JSON.stringify(row('L2', T0 + 901_000)));
  // Past the flush latency, inside the join hold.
  await vi.advanceTimersByTimeAsync(300);
  const slice = () => useChatConnectionStore.getState().channels.get('openroom');
  return { socket, slice, messages: () => ids(slice()?.messages ?? []) };
}

describe('chat open', () => {
  it('commits the newest 30 history rows first, then the older ones a frame later', async () => {
    const { messages } = await openWithLiveRows();
    historyGate.resolve(historyRows(100));
    await vi.advanceTimersByTimeAsync(0);

    expect(messages()).toEqual([...range(71, 100), 'L1', 'L2']);

    await vi.advanceTimersByTimeAsync(50);
    expect(messages()).toEqual([...range(1, 100), 'L1', 'L2']);
  });

  it('commits a short history in one step', async () => {
    const { messages } = await openWithLiveRows();
    historyGate.resolve(historyRows(20));
    await vi.advanceTimersByTimeAsync(0);

    expect(messages()).toEqual([...range(1, 20), 'L1', 'L2']);
    await vi.advanceTimersByTimeAsync(50);
    expect(messages()).toEqual([...range(1, 20), 'L1', 'L2']);
  });

  it('adds nothing when the channel is released before the older rows land', async () => {
    const { slice, messages } = await openWithLiveRows();
    historyGate.resolve(historyRows(100));
    await vi.advanceTimersByTimeAsync(0);
    expect(messages()).toEqual([...range(71, 100), 'L1', 'L2']);

    const released = slice();
    if (!released) throw new Error('no slice');
    const { releaseChannel } = await import('./chatConnectionStore');
    await releaseChannel('openroom', 'twitch');
    await vi.advanceTimersByTimeAsync(50);

    expect(ids(released.messages)).toEqual([...range(71, 100), 'L1', 'L2']);
    expect(slice()).toBeUndefined();
  });
});

describe('a backfill after an outage', () => {
  it('inserts its rows in one step', async () => {
    const { acquireChannel, useChatConnectionStore } = await import('./chatConnectionStore');
    const opening = acquireChannel('openroom', '42', 'twitch');
    await vi.advanceTimersByTimeAsync(50);
    await opening;
    const socket = FakeSocket.opened.at(-1);
    if (!socket) throw new Error('no socket opened');
    historyGate.resolve([]);
    socket.frame(JSON.stringify(row('A', T0 + 1_000)));
    socket.frame(JSON.stringify(row('B', T0 + 2_000)));
    await vi.advanceTimersByTimeAsync(2_000);
    const messages = () => ids(useChatConnectionStore.getState().channels.get('openroom')?.messages ?? []);
    expect(messages()).toEqual(['A', 'B']);

    socket.frame('PARK_GAP:openroom');
    socket.frame(JSON.stringify(row('F', T0 + 90_000)));
    socket.frame('PARK_END:openroom');
    await vi.advanceTimersByTimeAsync(300);

    const gap = Array.from({ length: 40 }, (_, i) => row(`G${String(i + 1).padStart(2, '0')}`, T0 + 3_000 + i * 1_000));
    backfillGate.resolve(gap);
    await vi.advanceTimersByTimeAsync(0);

    expect(messages()).toEqual(['A', 'B', ...gap.map((g) => g.id), 'F']);
  });
});
