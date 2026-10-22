import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// IRC says a chatter was timed out but not by whom; a moderator's EventSub
// entry (`room-mod-log`) names who acted and why. The two land in either order
// and are joined for ten seconds. These drive the real store with the socket
// and the Tauri event faked.

const invokeMock = vi.fn();
const handlers: Record<string, (e: { payload: unknown }) => void> = {};
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, cb: (e: { payload: unknown }) => void) => {
    handlers[name] = cb;
    return () => {};
  }),
  emit: vi.fn(),
}));
const settings = { moderation: { show_mod_messages: true } };
vi.mock('./AppStore', () => ({
  useAppStore: {
    getState: () => ({ settings, currentUser: null, modLogs: [], addModLog: () => {} }),
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
  frame(data: string) {
    this.onmessage?.({ data });
  }
}

const ROOM = 'modroom';

beforeEach(async () => {
  vi.useFakeTimers();
  vi.stubGlobal('WebSocket', FakeSocket);
  FakeSocket.opened = [];
  settings.moderation.show_mod_messages = true;
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (command: string) => {
    if (command === 'start_chat' || command === 'chat_bridge_port') return 4242;
    if (command === 'load_channel_history') return [];
    return undefined;
  });
  const { resetModJoinsForTest } = await import('./chatConnectionStore');
  resetModJoinsForTest();
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

async function openRoom() {
  const { acquireChannel } = await import('./chatConnectionStore');
  const opening = acquireChannel(ROOM, '42', 'twitch');
  await settle();
  await opening;
}

function socket(): FakeSocket {
  const s = FakeSocket.opened.at(-1);
  if (!s) throw new Error('no socket opened');
  return s;
}

function clearChat(extra: Record<string, unknown> = { ban_duration: 600 }) {
  socket().frame(
    JSON.stringify({ type: 'CLEARCHAT', channel: ROOM, target_user_id: '7', target_user: 'gus', ...extra }),
  );
}

function modLog(entry: Record<string, unknown>) {
  const handler = handlers['room-mod-log'];
  if (!handler) throw new Error('room-mod-log is not being listened to');
  handler({
    payload: {
      id: `x-${Math.random()}`,
      channel: ROOM,
      action: 'timeout',
      moderator: 'kay_lee',
      target: 'gus',
      target_id: '7',
      line: 'kay_lee timed out gus for 10m',
      quote: 'spoilers',
      reason: 'spoilers',
      message_id: null,
      at_ms: Date.now(),
      liftable: true,
      ...entry,
    },
  });
}

async function slice() {
  const { useChatConnectionStore } = await import('./chatConnectionStore');
  const s = useChatConnectionStore.getState().channels.get(ROOM);
  if (!s) throw new Error('no slice');
  return s;
}

async function systemRows(): Promise<{ id: string; content: string; segments: unknown[] }[]> {
  const s = await slice();
  return s.messages.filter((m) => typeof m !== 'string' && m.username === 'System');
}

describe('naming the moderator on a timeout', () => {
  it('IRC first: the struck rows and the line gain the name when EventSub lands', async () => {
    await openRoom();
    clearChat();
    const before = await slice();
    const contextsBefore = before.clearedUserContexts;
    const ctxBefore = contextsBefore.get('7')!.context;
    expect(ctxBefore.by).toBeUndefined();
    const [row] = await systemRows();
    expect(row.content).toBe('gus was timed out for 10m.');
    const segmentsBefore = row.segments;

    await vi.advanceTimersByTimeAsync(3_000);
    modLog({});

    const after = await slice();
    expect(after.clearedUserContexts).not.toBe(contextsBefore);
    const ctx = after.clearedUserContexts.get('7')!.context;
    expect(ctx).not.toBe(ctxBefore);
    expect(ctx).toMatchObject({ type: 'timeout', duration: 600, by: 'kay_lee', reason: 'spoilers' });
    const rows = await systemRows();
    expect(rows).toHaveLength(1);
    expect(rows[0].id).toBe(row.id);
    expect(rows[0].content).toBe('kay_lee timed out gus for 10m: spoilers');
    expect(rows[0].segments).not.toBe(segmentsBefore);
    expect(rows[0].segments).toEqual([{ type: 'text', content: 'kay_lee timed out gus for 10m: spoilers' }]);
  });

  it('EventSub first: CLEARCHAT writes the named line and the named context', async () => {
    await openRoom();
    modLog({});
    await vi.advanceTimersByTimeAsync(2_000);
    clearChat();

    const ctx = (await slice()).clearedUserContexts.get('7')!.context;
    expect(ctx).toMatchObject({ type: 'timeout', by: 'kay_lee', reason: 'spoilers' });
    const rows = await systemRows();
    expect(rows.map((r) => r.content)).toEqual(['kay_lee timed out gus for 10m: spoilers']);
  });

  it('a ban without a reason names only the moderator', async () => {
    await openRoom();
    modLog({ action: 'ban', line: 'kay_lee banned gus', reason: null, quote: null });
    clearChat({});
    const ctx = (await slice()).clearedUserContexts.get('7')!.context;
    expect(ctx.type).toBe('ban');
    expect(ctx.by).toBe('kay_lee');
    expect(ctx.reason).toBeUndefined();
    expect((await systemRows()).map((r) => r.content)).toEqual(['kay_lee banned gus']);
  });

  it('a waiting EventSub entry expires after ten seconds', async () => {
    await openRoom();
    modLog({});
    await vi.advanceTimersByTimeAsync(11_000);
    clearChat();
    expect((await slice()).clearedUserContexts.get('7')!.context.by).toBeUndefined();
    expect((await systemRows()).map((r) => r.content)).toEqual(['gus was timed out for 10m.']);
  });

  it('an IRC action stops waiting after ten seconds', async () => {
    await openRoom();
    clearChat();
    await vi.advanceTimersByTimeAsync(11_000);
    modLog({});
    expect((await slice()).clearedUserContexts.get('7')!.context.by).toBeUndefined();
    expect((await systemRows()).map((r) => r.content)).toEqual(['gus was timed out for 10m.']);
  });

  it('a timeout entry never names a ban', async () => {
    await openRoom();
    modLog({});
    clearChat({});
    expect((await slice()).clearedUserContexts.get('7')!.context.by).toBeUndefined();
  });

  it('names the struck rows even with the system lines off', async () => {
    settings.moderation.show_mod_messages = false;
    await openRoom();
    clearChat();
    modLog({});
    expect((await slice()).clearedUserContexts.get('7')!.context.by).toBe('kay_lee');
    expect(await systemRows()).toHaveLength(0);
  });
});

describe('rows only EventSub reports', () => {
  it('a warning gets its own line; a mode change does not (NOTICE writes that one)', async () => {
    await openRoom();
    modLog({ action: 'warn', line: 'kay_lee warned gus', reason: 'be nice', liftable: false });
    modLog({ action: 'slow', line: 'kay_lee turned on slow mode (30s)', reason: null, target_id: null });
    expect((await systemRows()).map((r) => r.content)).toEqual(['kay_lee warned gus: be nice']);
  });

  it('writes nothing with the system lines off', async () => {
    settings.moderation.show_mod_messages = false;
    await openRoom();
    modLog({ action: 'unban', line: 'kay_lee unbanned gus', reason: null, liftable: false });
    expect(await systemRows()).toHaveLength(0);
  });

  it('a delete names the line CLEARMSG wrote instead of adding one', async () => {
    await openRoom();
    socket().frame(
      JSON.stringify({ type: 'CLEARMSG', channel: ROOM, target_msg_id: 'm-1', login: 'gus', message: 'the boss dies' }),
    );
    modLog({ action: 'delete', line: 'kay_lee deleted a message from gus', reason: null, message_id: 'm-1' });
    expect((await systemRows()).map((r) => r.content)).toEqual(['kay_lee deleted a message from gus']);
  });
});

describe('replaceSystemMessage', () => {
  it('rewords the row with a new segments array', async () => {
    const { injectSystemMessage, replaceSystemMessage } = await import('./chatConnectionStore');
    await openRoom();
    const id = injectSystemMessage(ROOM, 'before');
    const before = (await systemRows())[0];
    expect(replaceSystemMessage(ROOM, id, 'after')).toBe(true);
    const after = (await systemRows())[0];
    expect(after.content).toBe('after');
    expect(after).not.toBe(before);
    expect(after.segments).not.toBe(before.segments);
    expect(replaceSystemMessage(ROOM, 'missing', 'x')).toBe(false);
  });
});
