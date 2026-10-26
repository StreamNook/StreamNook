import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// A chat tab that is not shown may lose its page copy of the channel's emotes
// (released after three minutes) or miss a 7TV edit. Showing the tab again
// reloads the set from Rust's table, which every edit patched.

const invokeMock = vi.fn();
const fetchAllEmotesMock = vi.hoisted(() => vi.fn());
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
  fetchAllEmotes: fetchAllEmotesMock,
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
  readyState = FakeSocket.CONNECTING;
  onopen: ((e: unknown) => void) | null = null;
  onmessage: ((e: unknown) => void) | null = null;
  onerror: ((e: unknown) => void) | null = null;
  onclose: ((e: unknown) => void) | null = null;
  constructor(public url: string) {
    setTimeout(() => {
      this.readyState = FakeSocket.OPEN;
      this.onopen?.({});
    }, 0);
  }
  send() {}
  close() {
    this.readyState = FakeSocket.CLOSED;
  }
}

const RELEASE_MS = 3 * 60_000;

function emote(id: string, name: string) {
  return { id, name, url: `https://cdn/${id}`, provider: '7tv' as const };
}

function setWith(...emotes: ReturnType<typeof emote>[]) {
  return { twitch: [], bttv: [], '7tv': emotes, ffz: [], kick: [], youtube: [] };
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal('WebSocket', FakeSocket);
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (command: string) => {
    if (command === 'start_chat' || command === 'chat_bridge_port') return 4242;
    if (command === 'load_channel_history') return [];
    return undefined;
  });
  fetchAllEmotesMock.mockReset();
});

afterEach(async () => {
  const { useChatConnectionStore, releaseChannel } = await import('./chatConnectionStore');
  for (const slice of Array.from(useChatConnectionStore.getState().channels.values())) {
    for (let i = 0; i < slice.refCount; i += 1) {
      await releaseChannel(slice.channel, slice.provider, { background: i < slice.backgroundRefs });
    }
  }
  await vi.advanceTimersByTimeAsync(20_000);
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

async function settle() {
  for (let i = 0; i < 10; i += 1) await vi.advanceTimersByTimeAsync(250);
}

/** Open the channel as a background hold (held back) with its emotes loaded. */
async function openParked(channel: string) {
  const store = await import('./chatConnectionStore');
  const opening = store.acquireChannel(channel, '42', 'twitch', { background: true });
  await settle();
  await opening;
  expect(store.useChatConnectionStore.getState().channels.get(channel)?.heldBack).toBe(true);
  fetchAllEmotesMock.mockResolvedValue(setWith(emote('a', 'Old')));
  await store.ensureChannelEmotes(channel, '42', 'twitch');
  expect(fetchAllEmotesMock).toHaveBeenCalledTimes(1);
  return store;
}

describe('a chat tab shown again', () => {
  it('reloads an emote set that was released while it was parked', async () => {
    const store = await openParked('tabroom1');

    await vi.advanceTimersByTimeAsync(RELEASE_MS + 1_000);
    expect(store.getChannelEmotes('tabroom1')).toBeNull();

    // A 7TV edit lands while the page holds no copy: nothing to patch, and the
    // table in Rust carries it.
    expect(store.applyChannelEmoteDelta('tabroom1', 'twitch', { added: [emote('b', 'Fresh')], removed: [] })).toBe(
      false,
    );
    fetchAllEmotesMock.mockResolvedValue(setWith(emote('a', 'Old'), emote('b', 'Fresh')));

    await store.acquireChannel('tabroom1', '42', 'twitch');
    await settle();

    expect(fetchAllEmotesMock).toHaveBeenCalledTimes(2);
    expect(store.getChannelEmotes('tabroom1')?.['7tv'].map((e) => e.name)).toEqual(['Old', 'Fresh']);
  });

  it('reloads a cached copy that is still present', async () => {
    const store = await openParked('tabroom2');
    const notified = vi.fn();
    const unsubscribe = store.subscribeChannelEmotes('tabroom2', notified);

    fetchAllEmotesMock.mockResolvedValue(setWith(emote('a', 'Old'), emote('c', 'Newer')));
    await store.acquireChannel('tabroom2', '42', 'twitch');
    await settle();

    expect(fetchAllEmotesMock).toHaveBeenCalledTimes(2);
    expect(store.getChannelEmotes('tabroom2')?.['7tv'].map((e) => e.name)).toEqual(['Old', 'Newer']);
    expect(notified).toHaveBeenCalled();
    unsubscribe();
  });

  it('does not reload when the tab was never held back', async () => {
    const store = await import('./chatConnectionStore');
    const opening = store.acquireChannel('tabroom3', '42', 'twitch');
    await settle();
    await opening;
    fetchAllEmotesMock.mockResolvedValue(setWith(emote('a', 'Old')));
    await store.ensureChannelEmotes('tabroom3', '42', 'twitch');
    await store.acquireChannel('tabroom3', '42', 'twitch');
    await settle();

    expect(fetchAllEmotesMock).toHaveBeenCalledTimes(1);
  });
});
