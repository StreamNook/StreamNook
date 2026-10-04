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

function setLayout(layout: Partial<MultiNookLayout>) {
  appState.settings.multi_nook_layout = { mode: 'main_row', strip_share: 0.25, small_quality_cap: null, ...layout };
}

const ids = () => usemultiNookStore.getState().slots.map((s) => s.id);
const byId = (id: string) => usemultiNookStore.getState().slots.find((s) => s.id === id)!;

beforeEach(() => {
  invoke.mockReset();
  invoke.mockResolvedValue(undefined);
  appState.settings = {};
  usemultiNookStore.setState({ slots: [], maximizedSlotId: null, activeChatChannelId: null });
});

describe('makeMainSlot', () => {
  it('swaps the tile into the main spot and moves sound and chat to it', () => {
    usemultiNookStore.setState({
      slots: [slot('a', { isFocused: true, muted: false }), slot('b'), slot('c'), slot('d')],
    });
    usemultiNookStore.getState().makeMainSlot('c');
    expect(ids()).toEqual(['c', 'b', 'a', 'd']);
    expect(byId('c')).toMatchObject({ isFocused: true, muted: false });
    expect(byId('a')).toMatchObject({ isFocused: false, muted: true });
    expect(usemultiNookStore.getState().activeChatChannelId).toBe('twitch:c');
  });

  it('skips a docked first tile: main is the first VISIBLE tile', () => {
    usemultiNookStore.setState({ slots: [slot('docked', { isMinimized: true }), slot('a'), slot('b')] });
    usemultiNookStore.getState().makeMainSlot('b');
    expect(ids()).toEqual(['docked', 'b', 'a']);
  });

  it('keeps the main tile focused when it is made main again', () => {
    usemultiNookStore.setState({ slots: [slot('a', { isFocused: true, muted: false }), slot('b')] });
    usemultiNookStore.getState().makeMainSlot('a');
    expect(byId('a')).toMatchObject({ isFocused: true, muted: false });
  });
});

describe('reconcileTileCaps', () => {
  const loaded = (id: string, over: Partial<MultiNookSlot> = {}) => slot(id, { streamUrl: `http://localhost/${id}`, ...over });

  it('does nothing without a cap', async () => {
    setLayout({ small_quality_cap: null });
    usemultiNookStore.setState({ slots: [loaded('a'), loaded('b')] });
    await usemultiNookStore.getState().reconcileTileCaps();
    expect(invoke).not.toHaveBeenCalled();
  });

  it('swaps small tiles in place and keeps their players', async () => {
    setLayout({ small_quality_cap: 480 });
    usemultiNookStore.setState({ slots: [loaded('a'), loaded('b'), loaded('c')] });
    invoke.mockImplementation(async (cmd: string) => (cmd === 'retier_multi_nook_tile' ? 'swapped' : undefined));
    await usemultiNookStore.getState().reconcileTileCaps();
    const retiered = invoke.mock.calls.filter(([c]) => c === 'retier_multi_nook_tile').map(([, a]) => a);
    expect(retiered.map((a) => a.streamId).sort()).toEqual(['b', 'c']);
    expect(retiered.every((a) => a.small === true)).toBe(true);
    expect(byId('b')).toMatchObject({ startedCap: 480, streamUrl: 'http://localhost/b' });
    expect(byId('a').startedCap ?? null).toBeNull();
    expect(invoke.mock.calls.some(([c]) => c === 'stop_multi_nook')).toBe(false);
  });

  it('restarts a tile Rust cannot swap, and one whose swap fails', async () => {
    setLayout({ small_quality_cap: 480 });
    usemultiNookStore.setState({ slots: [loaded('a'), loaded('b'), loaded('c')] });
    invoke.mockImplementation(async (cmd: string, a: { streamId: string }) => {
      if (cmd !== 'retier_multi_nook_tile') return undefined;
      if (a.streamId === 'b') return 'restart';
      throw new Error('resolve failed');
    });
    await usemultiNookStore.getState().reconcileTileCaps();
    const stopped = invoke.mock.calls.filter(([c]) => c === 'stop_multi_nook').map(([, a]) => a.streamId).sort();
    expect(stopped).toEqual(['b', 'c']);
    expect(byId('b').streamUrl).toBeUndefined();
    expect(byId('c').streamUrl).toBeUndefined();
    expect(byId('a').streamUrl).toBe('http://localhost/a');
  });

  it('lifts the cap from a tile that becomes main or is spotlighted', async () => {
    setLayout({ small_quality_cap: 480 });
    usemultiNookStore.setState({
      slots: [loaded('a', { startedCap: 480 }), loaded('b', { startedCap: 480 })],
      maximizedSlotId: 'b',
    });
    invoke.mockImplementation(async (cmd: string) => (cmd === 'retier_multi_nook_tile' ? 'swapped' : undefined));
    await usemultiNookStore.getState().reconcileTileCaps();
    const calls = invoke.mock.calls.filter(([c]) => c === 'retier_multi_nook_tile').map(([, a]) => [a.streamId, a.small]);
    expect(calls.sort()).toEqual([['a', false], ['b', false]]);
    expect(byId('a').startedCap).toBeNull();
    expect(byId('b').startedCap).toBeNull();
  });

  it('caps nothing in Grid', async () => {
    setLayout({ mode: 'grid', small_quality_cap: 480 });
    usemultiNookStore.setState({ slots: [loaded('a'), loaded('b')] });
    await usemultiNookStore.getState().reconcileTileCaps();
    expect(invoke).not.toHaveBeenCalled();
  });
});
