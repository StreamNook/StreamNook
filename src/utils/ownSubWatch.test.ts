// @vitest-environment node
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn();
const addToast = vi.fn();
const refreshChannelState = vi.fn(async () => {});

vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock('../stores/channelStateStore', () => ({
  refreshChannelState: (...args: unknown[]) => refreshChannelState(...(args as [])),
}));
vi.mock('../stores/AppStore', () => ({ useAppStore: { getState: () => ({ addToast }) } }));
vi.mock('./platform', () => ({ IS_MOBILE: false }));

import {
  OWN_SUB_SETTLED_EVENT,
  SETTLE_AGAIN_MS,
  SUB_POLL_MS,
  onOwnSubSettled,
  pollUntilSubscribed,
  settleOwnSub,
  stopOwnSubWatch,
  subscribePanelLogin,
  watchSubscribePage,
} from './ownSubWatch';

/** Lets the poll's awaited read settle between timer steps. */
const flush = async () => {
  for (let i = 0; i < 5; i += 1) await Promise.resolve();
};

// Node's own EventTarget and CustomEvent stand in for the window's.
vi.stubGlobal('window', new EventTarget());

beforeEach(() => {
  vi.useFakeTimers();
  invoke.mockReset();
  addToast.mockReset();
  refreshChannelState.mockClear();
});

afterEach(() => {
  stopOwnSubWatch('cloakzy');
  vi.useRealTimers();
});

describe('subscribePanelLogin', () => {
  it('reads the login out of a desktop subscribe panel label', () => {
    expect(subscribePanelLogin('subscribe-cloakzy-1794514099130')).toBe('cloakzy');
    expect(subscribePanelLogin('subscribe-Some_User-12')).toBe('some_user');
  });

  it('ignores every other panel', () => {
    expect(subscribePanelLogin('youtube-switch-1794514099130')).toBeNull();
    expect(subscribePanelLogin('subscribe-cloakzy')).toBeNull();
  });
});

describe('pollUntilSubscribed', () => {
  it('keeps asking until the sub shows, then calls back once', async () => {
    const answers = [false, false, true];
    const read = vi.fn(async () => answers.shift() ?? true);
    const done = vi.fn();
    pollUntilSubscribed(read, done, 100, 10);
    for (let i = 0; i < 5; i += 1) {
      await vi.advanceTimersByTimeAsync(100);
      await flush();
    }
    expect(read).toHaveBeenCalledTimes(3);
    expect(done).toHaveBeenCalledTimes(1);
  });

  it('gives up after its tries, a failed read counting as one', async () => {
    const read = vi.fn(async () => {
      throw new Error('offline');
    });
    const done = vi.fn();
    pollUntilSubscribed(read, done, 100, 3);
    for (let i = 0; i < 6; i += 1) {
      await vi.advanceTimersByTimeAsync(100);
      await flush();
    }
    expect(read).toHaveBeenCalledTimes(3);
    expect(done).not.toHaveBeenCalled();
  });

  it('asks nothing more once stopped', async () => {
    const read = vi.fn(async () => false);
    const stop = pollUntilSubscribed(read, vi.fn(), 100, 10);
    await vi.advanceTimersByTimeAsync(100);
    await flush();
    stop();
    await vi.advanceTimersByTimeAsync(1000);
    expect(read).toHaveBeenCalledTimes(1);
  });
});

describe('settleOwnSub', () => {
  it('re-reads the relationship and tells that channel twice, the second after a delay', () => {
    const heard = vi.fn();
    const other = vi.fn();
    const stopA = onOwnSubSettled('Cloakzy', heard);
    const stopB = onOwnSubSettled('symfuhny', other);
    settleOwnSub('CLOAKZY', 'subscribe-cloakzy-1');
    expect(refreshChannelState).toHaveBeenCalledWith('cloakzy', 'relationship');
    expect(heard).toHaveBeenCalledTimes(1);
    expect(heard.mock.calls[0][0]).toEqual({ channel: 'cloakzy', closedLabel: 'subscribe-cloakzy-1' });
    vi.advanceTimersByTime(SETTLE_AGAIN_MS);
    expect(heard).toHaveBeenCalledTimes(2);
    expect(heard.mock.calls[1][0]).toEqual({ channel: 'cloakzy' });
    expect(refreshChannelState).toHaveBeenCalledTimes(2);
    expect(other).not.toHaveBeenCalled();
    stopA();
    stopB();
  });

  it('dispatches the shared window event', () => {
    const seen: unknown[] = [];
    const listener = (e: Event) => seen.push((e as CustomEvent).detail);
    window.addEventListener(OWN_SUB_SETTLED_EVENT, listener);
    settleOwnSub('cloakzy');
    window.removeEventListener(OWN_SUB_SETTLED_EVENT, listener);
    expect(seen).toEqual([{ channel: 'cloakzy' }]);
  });
});

describe('watchSubscribePage', () => {
  it('a sub Twitch never posts in chat still thanks you, closes the page and re-reads', async () => {
    invoke.mockImplementation(async (cmd: string) =>
      cmd === 'channel_relationship' ? { subscribed: invoke.mock.calls.length > 1 } : undefined,
    );
    const heard = vi.fn();
    const stop = onOwnSubSettled('cloakzy', heard);
    watchSubscribePage('cloakzy', '81687332', 'subscribe-cloakzy-9');
    for (let i = 0; i < 3; i += 1) {
      await vi.advanceTimersByTimeAsync(SUB_POLL_MS);
      await flush();
    }
    expect(invoke).toHaveBeenCalledWith('channel_relationship', { channelId: '81687332', channelLogin: 'cloakzy' });
    expect(addToast).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith('close_login_overlay', { label: 'subscribe-cloakzy-9' });
    expect(heard).toHaveBeenCalled();
    stop();
  });

  it('closing the page stops the watch', async () => {
    invoke.mockResolvedValue({ subscribed: false });
    watchSubscribePage('cloakzy', '81687332', 'subscribe-cloakzy-9');
    settleOwnSub('cloakzy', 'subscribe-cloakzy-9');
    await vi.advanceTimersByTimeAsync(SUB_POLL_MS * 3);
    await flush();
    expect(invoke).not.toHaveBeenCalledWith('channel_relationship', expect.anything());
    expect(addToast).not.toHaveBeenCalled();
  });
});
