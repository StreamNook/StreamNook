import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { MultiNookSlot } from '../types';

// Rust is the other side of every call here; each test says what it answers.
const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }));
const appState = { settings: {}, addToast: vi.fn() };
vi.mock('./AppStore', () => ({
  useAppStore: Object.assign(vi.fn(), {
    getState: () => appState,
    setState: vi.fn(),
    subscribe: vi.fn(),
  }),
}));
vi.mock('../utils/settingsBroadcast', () => ({ patchSettings: vi.fn().mockResolvedValue(undefined) }));

import { usemultiNookStore } from './multiNookStore';

function slot(id: string, over: Partial<MultiNookSlot> = {}): MultiNookSlot {
  return { id, channelLogin: 'shared', volume: 1, muted: true, isFocused: false, ...over };
}

const byId = (id: string) => usemultiNookStore.getState().slots.find((s) => s.id === id)!;

beforeEach(() => {
  invoke.mockReset();
  invoke.mockResolvedValue(undefined);
  usemultiNookStore.setState({ slots: [], maximizedSlotId: null, activeChatChannelId: null });
});

describe('markSlotEnded', () => {
  it('stops a playing Kick tile and shows its offline card', () => {
    usemultiNookStore.setState({
      slots: [slot('kick-1', { provider: 'kick', streamUrl: 'http://localhost:1/stream.m3u8' })],
    });
    usemultiNookStore.getState().markSlotEnded('kick-1');
    expect(byId('kick-1')).toMatchObject({ offline: true, loadError: true, streamUrl: undefined });
    expect(invoke).toHaveBeenCalledWith('stop_multi_nook', { streamId: 'kick-1' });
  });

  it('leaves a Twitch tile with the same login untouched', () => {
    const playing = { streamUrl: 'http://localhost:2/stream.m3u8', offline: false };
    usemultiNookStore.setState({
      slots: [
        slot('kick-1', { provider: 'kick', streamUrl: 'http://localhost:1/stream.m3u8' }),
        slot('twitch-1', playing),
      ],
    });
    usemultiNookStore.getState().markSlotEnded('kick-1');
    expect(byId('twitch-1')).toMatchObject(playing);
    expect(byId('twitch-1').loadError).toBeUndefined();
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith('stop_multi_nook', { streamId: 'kick-1' });
  });

  it('does nothing for a slot that is gone or already showing the offline card', () => {
    usemultiNookStore.setState({
      slots: [slot('kick-1', { provider: 'kick', offline: true, loadError: true })],
    });
    const before = usemultiNookStore.getState().slots;
    usemultiNookStore.getState().markSlotEnded('kick-1');
    usemultiNookStore.getState().markSlotEnded('missing');
    expect(usemultiNookStore.getState().slots).toBe(before);
    expect(invoke).not.toHaveBeenCalled();
  });
});
