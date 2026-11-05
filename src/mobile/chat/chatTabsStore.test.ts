// The tab rules are Rust's (services/chat_tabs.rs, with its own tests). This
// store only mirrors Rust's answers and applies the connections they ask for.
import { describe, it, expect, vi, beforeEach } from 'vitest';

const acquire = vi.fn().mockResolvedValue(undefined);
const release = vi.fn().mockResolvedValue(undefined);

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

let tabEvent: ((e: { payload: unknown }) => void) | null = null;
vi.mock('@tauri-apps/api/event', () => ({
  listen: (_name: string, cb: (e: { payload: unknown }) => void) => {
    tabEvent = cb;
    return Promise.resolve(() => {});
  },
}));

vi.mock('../../stores/chatConnectionStore', () => ({
  acquireChannel: (...a: unknown[]) => acquire(...a),
  releaseChannel: (...a: unknown[]) => release(...a),
}));

import { useChatTabsStore } from './chatTabsStore';

const tab = (channel: string, provider: string, login: string, id: string | null, pinned = false) => ({
  channel,
  provider,
  login,
  channel_id: id,
  label: login,
  avatar: null,
  pinned_to_stream: pinned,
});

beforeEach(() => {
  acquire.mockClear();
  release.mockClear();
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
  useChatTabsStore.setState({ tabs: [], activeChannel: null, revision: 0, reloadNonce: {} });
});

describe('chat tabs mirror Rust', () => {
  it('asks Rust to open a room and joins what it answers', async () => {
    const kick = tab('kick:xqc', 'kick', 'xqc', null);
    invokeMock.mockResolvedValue({ tabs: [kick], active_channel: 'kick:xqc', mixed_platforms: false, revision: 1, acquire: [kick], release: [] });
    useChatTabsStore.getState().addTab('XQC', '676', 'xQc', null, 'kick');
    expect(invokeMock).toHaveBeenCalledWith('chat_tabs_add', {
      room: { provider: 'kick', channel: 'XQC', channel_id: '676', label: 'xQc', avatar: null },
    });
    await vi.waitFor(() => expect(acquire).toHaveBeenCalledWith('xqc', null, 'kick'));
    expect(useChatTabsStore.getState().tabs[0]).toMatchObject({ channel: 'kick:xqc', channelId: null, pinnedToStream: false });
    expect(useChatTabsStore.getState().activeChannel).toBe('kick:xqc');
  });

  it('releases what a stream switch leaves and joins the new room', async () => {
    const old = tab('kick:xqc', 'kick', 'xqc', null, true);
    const next = tab('xqc', 'twitch', 'xqc', '71092938', true);
    invokeMock.mockResolvedValue({ tabs: [next], active_channel: 'xqc', mixed_platforms: false, revision: 2, acquire: [next], release: [old] });
    useChatTabsStore.getState().syncStreamTab('xqc', '71092938', 'xQc', null, 'twitch');
    await vi.waitFor(() => expect(release).toHaveBeenCalledWith('xqc', 'kick'));
    expect(acquire).toHaveBeenLastCalledWith('xqc', '71092938', 'twitch');
    expect(useChatTabsStore.getState().tabs).toHaveLength(1);
  });

  it('drops the stream tab with a null room', () => {
    useChatTabsStore.getState().syncStreamTab(null, null, '');
    expect(invokeMock).toHaveBeenCalledWith('chat_tabs_sync_stream', { room: null });
  });

  it('never paints an older answer over a newer one', async () => {
    const a = tab('a', 'twitch', 'a', '1');
    const b = tab('b', 'twitch', 'b', '2');
    useChatTabsStore.setState({ tabs: [], activeChannel: null, revision: 5 });
    invokeMock.mockResolvedValue({ tabs: [a], active_channel: 'a', mixed_platforms: false, revision: 4, acquire: [], release: [] });
    useChatTabsStore.getState().removeTab('b');
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalled());
    expect(useChatTabsStore.getState().tabs).toEqual([]);
    tabEvent?.({ payload: { tabs: [a, b], active_channel: 'b', mixed_platforms: false, revision: 6 } });
    expect(useChatTabsStore.getState().tabs.map((t) => t.channel)).toEqual(['a', 'b']);
  });

  it('follows Rust patching a tab on its own', () => {
    useChatTabsStore.getState().setActive('a');
    const joe = { ...tab('kick:joewo', 'kick', 'joewo', null), label: 'JoeWo', avatar: 'https://x/joe.png' };
    tabEvent?.({ payload: { tabs: [joe], active_channel: 'kick:joewo', mixed_platforms: false, revision: 9 } });
    expect(useChatTabsStore.getState().tabs[0]).toMatchObject({ label: 'JoeWo', avatar: 'https://x/joe.png' });
  });
});

describe('reload', () => {
  it('reloads a Kick room at the adapter, not the ref-counted store', async () => {
    const kick = { ...tab('kick:xqc', 'kick', 'xqc', null), label: 'xQc', avatar: 'a.png' };
    useChatTabsStore.getState().setActive('kick:xqc');
    tabEvent?.({ payload: { tabs: [kick], active_channel: 'kick:xqc', mixed_platforms: false, revision: 10 } });
    invokeMock.mockClear();
    useChatTabsStore.getState().reload('kick:xqc');
    await vi.waitFor(() =>
      expect(invokeMock).toHaveBeenLastCalledWith('provider_chat_connect', { provider: 'kick', channel: 'xqc' }),
    );
    expect(invokeMock.mock.calls[0]).toEqual(['provider_chat_disconnect', { provider: 'kick', channel: 'xqc' }]);
    expect(release).not.toHaveBeenCalled();
    expect(acquire).not.toHaveBeenCalled();
    expect(useChatTabsStore.getState().reloadNonce['kick:xqc']).toBe(1);
  });
});

describe('keep, load and order', () => {
  const view = (tabs: unknown[], revision: number, release: unknown[] = []) => ({
    tabs,
    active_channel: null,
    mixed_platforms: false,
    revision,
    acquire: [],
    release,
  });

  it('keep answers true once Rust shows the room kept, and mirrors it', async () => {
    const s = { ...tab('s', 'twitch', 's', '2', true), kept: true };
    invokeMock.mockResolvedValue({ ...view([s], 3), active_channel: 's' });
    const kept = await useChatTabsStore.getState().keep('s', true);
    expect(kept).toBe(true);
    expect(invokeMock).toHaveBeenCalledWith('chat_tabs_keep', { channel: 's', keep: true });
    expect(useChatTabsStore.getState().tabs[0]).toMatchObject({ channel: 's', kept: true, pinnedToStream: true });
  });

  it('keep answers false when Rust did not keep it or the call failed', async () => {
    invokeMock.mockResolvedValueOnce(view([tab('s', 'twitch', 's', '2', true)], 4));
    expect(await useChatTabsStore.getState().keep('s', true)).toBe(false);
    invokeMock.mockRejectedValueOnce(new Error('down'));
    expect(await useChatTabsStore.getState().keep('s', true)).toBe(false);
  });

  it("load leaves the rooms another account's restore let go", async () => {
    invokeMock.mockResolvedValue(view([], 7, [tab('a', 'twitch', 'a', '1')]));
    useChatTabsStore.getState().load();
    await vi.waitFor(() => expect(release).toHaveBeenCalledWith('a', 'twitch'));
  });

  it('sends each command only after the one before it answered', async () => {
    let answerFirst: (v: unknown) => void = () => {};
    invokeMock.mockImplementationOnce(() => new Promise((r) => (answerFirst = r)));
    const first = useChatTabsStore.getState().keep('s', true);
    useChatTabsStore.getState().syncStreamTab(null, null, '');
    await Promise.resolve();
    expect(invokeMock).toHaveBeenCalledTimes(1);
    answerFirst(view([], 8));
    await first;
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(2));
    expect(invokeMock.mock.calls[1]).toEqual(['chat_tabs_sync_stream', { room: null }]);
  });
});

describe('the return from the background', () => {
  const parked = tab('a', 'twitch', 'a', '1');
  const answer = (acquire: unknown[]) => ({ tabs: [parked], active_channel: 'a', mixed_platforms: false, connection: 'connected', revision: 20, acquire, release: [] });

  it('asks Rust to join a parked list and settles once the answer is applied', async () => {
    invokeMock.mockResolvedValue(answer([parked]));
    await useChatTabsStore.getState().connect({ resume: true });
    expect(invokeMock).toHaveBeenCalledWith('chat_tabs_connect', { resume: true });
    expect(acquire).toHaveBeenCalledWith('a', '1', 'twitch');
  });

  it('the first look still joins without the resume flag', async () => {
    invokeMock.mockResolvedValue(answer([]));
    await useChatTabsStore.getState().connect();
    expect(invokeMock).toHaveBeenCalledWith('chat_tabs_connect', {});
  });

  it('waits for joins any tab change started, at most the cap', async () => {
    const { settleJoins } = await import('./chatTabsStore');
    let finish: () => void = () => {};
    acquire.mockImplementationOnce(() => new Promise<void>((r) => (finish = r)));
    invokeMock.mockResolvedValue({ tabs: [tab('s', 'twitch', 's', '2', true)], active_channel: 's', mixed_platforms: false, revision: 21, acquire: [tab('s', 'twitch', 's', '2', true)], release: [] });
    useChatTabsStore.getState().syncStreamTab('s', '2', 'S');
    await vi.waitFor(() => expect(acquire).toHaveBeenCalledWith('s', '2', 'twitch'));
    let settled = false;
    const waiting = settleJoins(5_000).then(() => (settled = true));
    await Promise.resolve();
    expect(settled).toBe(false);
    finish();
    await waiting;
    expect(settled).toBe(true);
  });

  it('also waits for a join started while it waits', async () => {
    const { settleJoins } = await import('./chatTabsStore');
    let finishA: () => void = () => {};
    let finishB: () => void = () => {};
    acquire.mockImplementationOnce(() => new Promise<void>((r) => (finishA = r)));
    acquire.mockImplementationOnce(() => new Promise<void>((r) => (finishB = r)));
    try {
      invokeMock.mockResolvedValue({ tabs: [], active_channel: null, mixed_platforms: false, revision: 23, acquire: [tab('p', 'twitch', 'p', '5')], release: [] });
      await useChatTabsStore.getState().connect({ resume: true });
      let settled = false;
      const waiting = settleJoins(5_000).then(() => (settled = true));
      invokeMock.mockResolvedValue({ tabs: [], active_channel: null, mixed_platforms: false, revision: 24, acquire: [tab('q', 'twitch', 'q', '6')], release: [] });
      await useChatTabsStore.getState().connect();
      finishA();
      // A full task turn, not a counted few microtasks: a wait that does not
      // loop would have settled by now.
      await new Promise((r) => setTimeout(r, 0));
      expect(settled).toBe(false);
      finishB();
      await waiting;
      expect(settled).toBe(true);
    } finally {
      finishA();
      finishB();
    }
  });

  it('stops waiting at the cap, and a failed join never rejects the wait', async () => {
    vi.useFakeTimers();
    let release: () => void = () => {};
    try {
      const { settleJoins } = await import('./chatTabsStore');
      acquire.mockImplementationOnce(() => new Promise<void>((r) => (release = r)));
      acquire.mockImplementationOnce(() => Promise.reject(new Error('join failed')));
      invokeMock.mockResolvedValue({ tabs: [], active_channel: null, mixed_platforms: false, revision: 22, acquire: [tab('x', 'twitch', 'x', '3'), tab('y', 'twitch', 'y', '4')], release: [] });
      await useChatTabsStore.getState().connect({ resume: true });
      let settled = false;
      const waiting = settleJoins(5_000).then(() => (settled = true));
      await vi.advanceTimersByTimeAsync(4_999);
      expect(settled).toBe(false);
      await vi.advanceTimersByTimeAsync(1);
      await waiting;
      expect(settled).toBe(true);
    } finally {
      // The never-answering join leaves the module's pending set, so no later
      // test waits on it.
      release();
      vi.useRealTimers();
    }
  });

  it('a failed connect still settles', async () => {
    invokeMock.mockImplementation(() => Promise.reject(new Error('core gone')));
    await expect(useChatTabsStore.getState().connect({ resume: true })).resolves.toBeUndefined();
  });
});
