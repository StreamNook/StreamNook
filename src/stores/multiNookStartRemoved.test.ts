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
  return { id, channelLogin: id, volume: 1, muted: true, isFocused: false, ...over };
}

const stops = () => invoke.mock.calls.filter(([c]) => c === 'stop_multi_nook');

beforeEach(() => {
  invoke.mockReset();
  invoke.mockResolvedValue(undefined);
  usemultiNookStore.setState({ slots: [], maximizedSlotId: null, activeChatChannelId: null });
});

describe('batchLoadMissingStreams', () => {
  it('stops the relay of a tile removed while its start was in flight', async () => {
    const finishStarts: Array<(url: string) => void> = [];
    invoke.mockImplementation((cmd: string) =>
      cmd === 'start_multi_nook'
        ? new Promise<string>((resolve) => {
            finishStarts.push(resolve);
          })
        : Promise.resolve(undefined),
    );
    usemultiNookStore.setState({ slots: [slot('a'), slot('b')] });

    const loading = usemultiNookStore.getState().batchLoadMissingStreams();
    await usemultiNookStore.getState().removeSlot('a');
    expect(stops()).toHaveLength(0);

    finishStarts.forEach((finish) => finish('http://localhost:1/stream.m3u8'));
    await loading;

    expect(stops()).toEqual([['stop_multi_nook', { streamId: 'a' }]]);
    expect(usemultiNookStore.getState().slots.map((s) => s.id)).toEqual(['b']);
  });

  it('keeps the url of a tile that is still there when its start lands', async () => {
    invoke.mockImplementation(async (cmd: string) =>
      cmd === 'start_multi_nook' ? 'http://localhost:1/stream.m3u8' : undefined,
    );
    usemultiNookStore.setState({ slots: [slot('a')] });

    await usemultiNookStore.getState().batchLoadMissingStreams();

    expect(stops()).toHaveLength(0);
    expect(usemultiNookStore.getState().slots[0].streamUrl).toBe('http://localhost:1/stream.m3u8');
  });
});
