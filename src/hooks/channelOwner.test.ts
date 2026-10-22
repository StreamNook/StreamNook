// @vitest-environment node
// Run with: npm test
//
// The main chat's hold on its channel (useTwitchChat's createChannelOwner).
// A close that lands while the first connect is still in flight must release
// the channel, or the slice leaks and the next open of it never connects; and
// StrictMode's replayed mount must take back what its cleanup let go.

import { beforeEach, describe, expect, it, vi } from 'vitest';

const store = vi.hoisted(() => {
  const pending: Array<() => void> = [];
  return {
    pending,
    acquireChannel: vi.fn(
      (_channel: string, _roomId: string | null) =>
        new Promise<void>((resolve) => {
          pending.push(resolve);
        }),
    ),
    releaseChannel: vi.fn(async (_channel: string) => {}),
  };
});

vi.mock('../stores/chatConnectionStore', () => ({
  acquireChannel: store.acquireChannel,
  releaseChannel: store.releaseChannel,
  sendChannelMessage: vi.fn(),
  setChannelPaused: vi.fn(),
  useChannelChatMeta: vi.fn(),
}));

import { createChannelOwner } from './useTwitchChat';

/** Acquires minus releases of `channel`: what the owner still holds of it. */
function held(channel: string): number {
  const acquired = store.acquireChannel.mock.calls.filter(([c]) => c === channel).length;
  const released = store.releaseChannel.mock.calls.filter(([c]) => c === channel).length;
  return acquired - released;
}

function resolveAll() {
  for (const resolve of store.pending.splice(0)) resolve();
}

beforeEach(() => {
  store.pending.length = 0;
  store.acquireChannel.mockClear();
  store.releaseChannel.mockClear();
});

describe('the main chat holds exactly the channel it shows', () => {
  it('releases a channel whose first connect is still in flight when it unmounts', async () => {
    const owner = createChannelOwner();
    const connecting = owner.connect('XQC', '71092938');
    owner.dispose();
    resolveAll();
    await connecting;

    expect(store.releaseChannel).toHaveBeenCalledWith('xqc');
    expect(held('xqc')).toBe(0);
    expect(owner.current()).toBeNull();
  });

  it('switching A, B, then back to A while B still connects keeps only A', async () => {
    const owner = createChannelOwner();
    const a = owner.connect('a', '1');
    resolveAll();
    await a;
    const b = owner.connect('b', '2');
    const backToA = owner.connect('a', '1');
    resolveAll();
    await Promise.all([b, backToA]);

    expect(owner.current()).toBe('a');
    expect(held('a')).toBe(1);
    expect(held('b')).toBe(0);
  });

  it('refreshing the room id of the held channel leaves the count unchanged', async () => {
    const owner = createChannelOwner();
    const first = owner.connect('xqc');
    resolveAll();
    await first;
    const again = owner.connect('xqc', '71092938');
    resolveAll();
    await again;

    expect(held('xqc')).toBe(1);
  });

  it("takes back the channel a StrictMode cleanup let go on the replayed mount", async () => {
    const owner = createChannelOwner();
    // setup: the widget connects; cleanup: the replay unmounts it mid-connect.
    const connecting = owner.connect('xqc', '71092938');
    owner.dispose();
    // setup again: nothing asks for the channel, the owner resumes it.
    const resumed = owner.resume();
    expect(resumed).not.toBeNull();
    resolveAll();
    await Promise.all([connecting, resumed]);

    expect(owner.current()).toBe('xqc');
    expect(held('xqc')).toBe(1);
    expect(store.acquireChannel).toHaveBeenLastCalledWith('xqc', '71092938');
  });

  it('a first mount has nothing to resume', () => {
    expect(createChannelOwner().resume()).toBeNull();
  });
});
