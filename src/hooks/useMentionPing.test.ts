import { beforeEach, describe, expect, it, vi } from 'vitest';

type Handler = (e: { payload: { channel: string; sound: string; volume: number } }) => void;

const m = vi.hoisted(() => ({
  handler: undefined as Handler | undefined,
  unlisten: vi.fn(),
  winListen: vi.fn(),
  emit: vi.fn(async () => {}),
  globalListen: vi.fn(),
  playSound: vi.fn(),
}));

vi.mock('@tauri-apps/api/webviewWindow', () => ({
  getCurrentWebviewWindow: () => ({ label: 'multichat-default', listen: m.winListen }),
}));
vi.mock('@tauri-apps/api/event', () => ({ emit: m.emit, listen: m.globalListen }));
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

  it('listens on this window only and tells Rust while it can play', async () => {
    const { startMentionPing } = await import('./useMentionPing');
    const stop = startMentionPing();
    await flush();
    expect(m.winListen).toHaveBeenCalledWith('chat-mention-ping', expect.any(Function));
    expect(m.globalListen).not.toHaveBeenCalled();
    expect(m.emit).toHaveBeenCalledWith('mention-ping-ready', { label: 'multichat-default', ready: true });
    m.handler?.({ payload: { channel: 'a', sound: 'boop', volume: 40 } });
    expect(m.playSound).toHaveBeenCalledWith('boop', 40);
    stop();
    expect(m.unlisten).toHaveBeenCalledTimes(1);
    expect(m.emit).toHaveBeenLastCalledWith('mention-ping-ready', { label: 'multichat-default', ready: false });
  });

  it('stopped before the listener lands: unlistens and never says ready', async () => {
    const { startMentionPing } = await import('./useMentionPing');
    const stop = startMentionPing();
    stop();
    await flush();
    expect(m.unlisten).toHaveBeenCalledTimes(1);
    expect(m.emit).not.toHaveBeenCalled();
  });
});
