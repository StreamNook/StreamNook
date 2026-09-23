import { describe, it, expect, vi, beforeEach } from 'vitest';

const acquire = vi.fn().mockResolvedValue(undefined);
const release = vi.fn().mockResolvedValue(undefined);

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

// Only the connection calls are faked; the key logic is the real one from
// utils/providerKey, so a change to the slice-key shape is caught here.
vi.mock('../../stores/chatConnectionStore', () => ({
  acquireChannel: (...a: unknown[]) => acquire(...a),
  releaseChannel: (...a: unknown[]) => release(...a),
}));

import { useChatTabsStore } from './chatTabsStore';

beforeEach(() => {
  acquire.mockClear();
  release.mockClear();
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
  useChatTabsStore.setState({ tabs: [], activeChannel: null, reloadNonce: {} });
});

describe('chat tabs are provider-keyed', () => {
  it('keys a Kick tab as kick:slug and acquires with provider kick and no id', () => {
    useChatTabsStore.getState().addTab('XQC', '676', 'xQc', null, 'kick');
    const t = useChatTabsStore.getState().tabs[0];
    expect(t).toMatchObject({ channel: 'kick:xqc', provider: 'kick', login: 'xqc', channelId: null });
    expect(acquire).toHaveBeenCalledWith('xqc', null, 'kick');
  });

  it('keeps Twitch tabs bare, as before', () => {
    useChatTabsStore.getState().addTab('Foo', '9', 'Foo');
    expect(useChatTabsStore.getState().tabs[0]).toMatchObject({ channel: 'foo', provider: 'twitch', channelId: '9' });
    expect(acquire).toHaveBeenCalledWith('foo', '9', 'twitch');
  });

  it('holds Twitch xqc and Kick xqc as two tabs', () => {
    const s = useChatTabsStore.getState();
    s.addTab('xqc', '1', 'xqc');
    s.addTab('xqc', null, 'xqc', null, 'kick');
    expect(useChatTabsStore.getState().tabs.map((t) => t.channel)).toEqual(['xqc', 'kick:xqc']);
  });

  it('releases a Kick tab with its provider', () => {
    useChatTabsStore.getState().addTab('xqc', null, 'xqc', null, 'kick');
    useChatTabsStore.getState().removeTab('kick:xqc');
    expect(release).toHaveBeenCalledWith('xqc', 'kick');
  });

  it('re-points the stream tab from Kick to Twitch across key spaces', () => {
    const s = useChatTabsStore.getState();
    s.syncStreamTab('xqc', '676', 'xqc', null, 'kick');
    s.syncStreamTab('xqc', '71092938', 'xQc', null, 'twitch');
    expect(release).toHaveBeenCalledWith('xqc', 'kick');
    expect(acquire).toHaveBeenLastCalledWith('xqc', '71092938', 'twitch');
    expect(useChatTabsStore.getState().tabs).toHaveLength(1);
    expect(useChatTabsStore.getState().tabs[0].channel).toBe('xqc');
  });
});

describe('Kick tab upkeep', () => {
  it('fills in the name and avatar of a tab opened from a bare slug', async () => {
    invokeMock.mockResolvedValue({ user_name: 'JoeWo', profile_image_url: 'https://x/joe.png' });
    useChatTabsStore.getState().addTab('joewo', null, 'joewo', null, 'kick');
    await vi.waitFor(() =>
      expect(useChatTabsStore.getState().tabs[0]).toMatchObject({ label: 'JoeWo', avatar: 'https://x/joe.png' }),
    );
    expect(invokeMock).toHaveBeenCalledWith('provider_channel_meta', { provider: 'kick', channel: 'joewo' });
  });

  it('does not ask again when the tab already has a name and picture', () => {
    useChatTabsStore.getState().addTab('joewo', null, 'JoeWo', 'https://x/joe.png', 'kick');
    expect(invokeMock).not.toHaveBeenCalledWith('provider_channel_meta', expect.anything());
  });

  it('reloads a Kick room at the adapter, not the ref-counted store', async () => {
    useChatTabsStore.getState().addTab('xqc', null, 'xQc', 'a.png', 'kick');
    invokeMock.mockClear();
    release.mockClear();
    acquire.mockClear();
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
