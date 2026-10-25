import { beforeEach, describe, expect, it, vi } from 'vitest';

type Ping = { id: number; channel: string; sound: string; volume: number };
type Handler = (e: { payload: Ping }) => void;

const m = vi.hoisted(() => ({
  handler: undefined as Handler | undefined,
  unlisten: vi.fn(),
  winListen: vi.fn(),
  invoke: vi.fn(),
  globalListen: vi.fn(),
  playSound: vi.fn(),
}));

vi.mock('@tauri-apps/api/webviewWindow', () => ({
  getCurrentWebviewWindow: () => ({ label: 'multichat-default', listen: m.winListen }),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: m.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: m.globalListen }));
vi.mock('../utils/notificationSound', () => ({ playSound: m.playSound }));

const flush = () => new Promise((r) => setTimeout(r, 0));
const ping: Ping = { id: 7, channel: 'a', sound: 'boop', volume: 40 };

describe('startMentionPing', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    m.winListen.mockImplementation(async (_event: string, cb: Handler) => {
      m.handler = cb;
      return m.unlisten;
    });
  });

  it('listens on this window only', async () => {
    const { startMentionPing } = await import('./useMentionPing');
    const stop = startMentionPing();
    await flush();
    expect(m.winListen).toHaveBeenCalledWith('chat-mention-ping', expect.any(Function));
    expect(m.globalListen).not.toHaveBeenCalled();
    stop();
    expect(m.unlisten).toHaveBeenCalledTimes(1);
  });

  it('plays a ping only when its claim is granted', async () => {
    const { startMentionPing } = await import('./useMentionPing');
    const stop = startMentionPing();
    await flush();
    m.invoke.mockResolvedValueOnce(true);
    m.handler?.({ payload: ping });
    await flush();
    expect(m.invoke).toHaveBeenCalledWith('mention_ping_claim', { id: 7 });
    expect(m.playSound).toHaveBeenCalledWith('boop', 40);

    m.playSound.mockClear();
    m.invoke.mockResolvedValueOnce(false);
    m.handler?.({ payload: { ...ping, id: 8 } });
    await flush();
    expect(m.playSound).not.toHaveBeenCalled();

    m.invoke.mockRejectedValueOnce(new Error('no app'));
    m.handler?.({ payload: { ...ping, id: 9 } });
    await flush();
    expect(m.playSound).not.toHaveBeenCalled();
    stop();
  });

  it('stopped before the listener lands: unlistens at once', async () => {
    const { startMentionPing } = await import('./useMentionPing');
    const stop = startMentionPing();
    stop();
    await flush();
    expect(m.unlisten).toHaveBeenCalledTimes(1);
  });
});
