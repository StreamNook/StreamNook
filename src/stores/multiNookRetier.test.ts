import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { MultiNookLayout, MultiNookSlot } from '../types';

// Rust is the other side of every call here; each test says what it answers.
const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }));
const appState: { settings: { multi_nook_layout?: MultiNookLayout }; addToast: () => void } = {
  settings: {},
  addToast: vi.fn(),
};
vi.mock('./AppStore', () => ({
  useAppStore: Object.assign(vi.fn(), {
    getState: () => appState,
    setState: vi.fn((fn: (s: typeof appState) => Partial<typeof appState>) => Object.assign(appState, fn(appState))),
    subscribe: vi.fn(),
  }),
}));
vi.mock('../utils/settingsBroadcast', () => ({ patchSettings: vi.fn().mockResolvedValue(undefined) }));

import { usemultiNookStore } from './multiNookStore';

function slot(id: string, over: Partial<MultiNookSlot> = {}): MultiNookSlot {
  return { id, channelLogin: id, volume: 1, muted: true, isFocused: false, ...over };
}

const starts = () => invoke.mock.calls.filter(([c]) => c === 'start_multi_nook');
const byId = (id: string) => usemultiNookStore.getState().slots.find((s) => s.id === id)!;

beforeEach(() => {
  invoke.mockReset();
  invoke.mockResolvedValue(undefined);
  appState.settings = { multi_nook_layout: { mode: 'main_row', strip_share: 0.25, small_quality_cap: 480 } };
  usemultiNookStore.setState({ slots: [], maximizedSlotId: null, activeChatChannelId: null });
});

describe('reconcileTileCaps', () => {
  it('starts a tile a recovery cleared while its quality change was running, once', async () => {
    let finishRetier: (outcome: 'swapped') => void = () => {};
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'retier_multi_nook_tile') {
        return new Promise<'swapped'>((resolve) => {
          finishRetier = resolve;
        });
      }
      if (cmd === 'start_multi_nook') return Promise.resolve('http://localhost/b-restarted');
      return Promise.resolve(undefined);
    });
    usemultiNookStore.setState({
      slots: [slot('a', { streamUrl: 'http://localhost/a' }), slot('b', { streamUrl: 'http://localhost/b' })],
    });

    const reconciling = usemultiNookStore.getState().reconcileTileCaps();
    // The tile is mid-retier; the loader skips it for the in-flight mark.
    usemultiNookStore.getState().retrySlot('b');
    await usemultiNookStore.getState().batchLoadMissingStreams();
    expect(starts()).toHaveLength(0);

    finishRetier('swapped');
    await reconciling;
    await vi.waitFor(() => expect(byId('b').streamUrl).toBe('http://localhost/b-restarted'));

    expect(starts()).toHaveLength(1);
    expect(starts()[0][1]).toMatchObject({ streamId: 'b', small: true });
  });
});
