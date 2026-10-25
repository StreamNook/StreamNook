import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

type Handler = (e: { payload: { channel: string; sound: string; volume: number } }) => void;

const m = vi.hoisted(() => ({
  handler: undefined as Handler | undefined,
  unlisten: vi.fn(),
  winListen: vi.fn(),
  invoke: vi.fn(async () => {}),
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

describe('startMentionPing', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    m.winListen.mockImplementation(async (_event: string, cb: Handler) => {
      m.handler = cb;
      return m.unlisten;
    });
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('listens on this window only and tells Rust while it can play', async () => {
    const { startMentionPing } = await import('./useMentionPing');
    const stop = startMentionPing();
    await flush();
    expect(m.winListen).toHaveBeenCalledWith('chat-mention-ping', expect.any(Function));
    expect(m.globalListen).not.toHaveBeenCalled();
    expect(m.invoke).toHaveBeenCalledWith('mention_ping_ready', { ready: true });
    m.handler?.({ payload: { channel: 'a', sound: 'boop', volume: 40 } });
    expect(m.playSound).toHaveBeenCalledWith('boop', 40);
    stop();
    expect(m.unlisten).toHaveBeenCalledTimes(1);
    expect(m.invoke).toHaveBeenLastCalledWith('mention_ping_ready', { ready: false });
  });

  it('stopped before the listener lands: unlistens and never says ready', async () => {
    const { startMentionPing } = await import('./useMentionPing');
    const stop = startMentionPing();
    stop();
    await flush();
    expect(m.unlisten).toHaveBeenCalledTimes(1);
    expect(m.invoke).not.toHaveBeenCalled();
  });

  it('a page going away says it stopped, and a stopped page never does', async () => {
    const win = new EventTarget();
    vi.stubGlobal('window', win);
    const { startMentionPing } = await import('./useMentionPing');
    const stop = startMentionPing();
    await flush();
    win.dispatchEvent(new Event('pagehide'));
    expect(m.invoke).toHaveBeenLastCalledWith('mention_ping_ready', { ready: false });
    stop();
    m.invoke.mockClear();
    win.dispatchEvent(new Event('pagehide'));
    expect(m.invoke).not.toHaveBeenCalled();
  });
});
