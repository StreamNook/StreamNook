import { beforeEach, describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({
  invoke: vi.fn(),
  startStream: vi.fn(),
  state: { isBooting: false, currentStream: null as { user_login?: string; provider?: string } | null },
  bootListeners: [] as Array<(s: { isBooting: boolean }) => void>,
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }));
vi.mock('../utils/logger', () => ({
  Logger: { debug: vi.fn(), info: vi.fn(), warn: vi.fn(), error: vi.fn() },
}));
vi.mock('./AppStore', () => ({
  useAppStore: {
    getState: () => ({ ...h.state, startStream: h.startStream }),
    subscribe: (fn: (s: { isBooting: boolean }) => void) => {
      h.bootListeners.push(fn);
      return () => {
        const at = h.bootListeners.indexOf(fn);
        if (at >= 0) h.bootListeners.splice(at, 1);
      };
    },
  },
}));

import { openMentionInMain } from './mentionJumpStore';

const entry = { id: 'm1', room: 'xqc', open_key: 'xqc' };

function endBoot() {
  h.state.isBooting = false;
  for (const fn of [...h.bootListeners]) fn({ isBooting: false });
}

beforeEach(() => {
  h.invoke.mockReset();
  h.startStream.mockReset();
  h.state.isBooting = false;
  h.state.currentStream = null;
  h.bootListeners.length = 0;
  vi.stubGlobal('window', {
    dispatchEvent: vi.fn(),
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  });
});

describe('openMentionInMain reports that it opened', () => {
  // Fails without the change: `opened` is never called, or is called only
  // after the awaited open_mention, so the count right after the call is 0.
  it('calls opened once, before open_mention answers', async () => {
    let answer!: (v: unknown) => void;
    h.invoke.mockReturnValue(new Promise((resolve) => { answer = resolve; }));
    const opened = vi.fn();
    const done = openMentionInMain('m1', 'xqc', opened);
    expect(opened).toHaveBeenCalledTimes(1);
    expect(h.invoke).toHaveBeenCalledWith('open_mention', { id: 'm1' });
    answer(entry);
    await done;
    expect(opened).toHaveBeenCalledTimes(1);
  });

  // Fails without the change for the same reason, and fails if a rejected
  // open_mention is made to skip the report (the channel still opens).
  it('reports when open_mention rejects, and still opens the channel', async () => {
    h.invoke.mockRejectedValue(new Error('gone'));
    const opened = vi.fn();
    await openMentionInMain('m1', 'xqc', opened);
    expect(opened).toHaveBeenCalledTimes(1);
    expect(h.startStream).toHaveBeenCalledWith('xqc');
  });

  // Fails if the report waits for the start: while booting the start is
  // deferred, and a late report would reach Rust after the post-boot take.
  it('reports before a start that boot defers, and starts after boot ends', async () => {
    h.state.isBooting = true;
    h.invoke.mockResolvedValue(entry);
    const opened = vi.fn();
    await openMentionInMain('m1', 'xqc', opened);
    expect(opened).toHaveBeenCalledTimes(1);
    expect(h.startStream).not.toHaveBeenCalled();
    endBoot();
    expect(h.startStream).toHaveBeenCalledWith('xqc');
    expect(opened).toHaveBeenCalledTimes(1);
  });

  // The chat already showing the room takes the jump; this is still an open.
  it('reports once for a room that is already playing and starts nothing', async () => {
    h.state.currentStream = { user_login: 'xqc', provider: 'twitch' };
    h.invoke.mockResolvedValue(entry);
    const opened = vi.fn();
    await openMentionInMain('m1', 'xqc', opened);
    expect(opened).toHaveBeenCalledTimes(1);
    expect(h.startStream).not.toHaveBeenCalled();
  });

  // The Island row passes no callback; the old two-argument call must work.
  it('works with no callback', async () => {
    h.invoke.mockResolvedValue(entry);
    await expect(openMentionInMain('m1', 'xqc')).resolves.toBeUndefined();
    expect(h.startStream).toHaveBeenCalledWith('xqc');
  });
});
