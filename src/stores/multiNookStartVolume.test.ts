import { describe, it, expect, vi, beforeEach } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import type { MultiNookSlot } from '../types';

// Same isolation as multiNookIdentity.test.ts, with the app store's settings
// reachable so each case can set the player's Start volume.
const app = vi.hoisted(() => ({
  state: { addToast: () => {}, settings: {} as Record<string, unknown>, currentStream: null, updateSettings: () => {} },
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(null) }));
vi.mock('./AppStore', () => ({
  useAppStore: Object.assign(vi.fn(), { getState: () => app.state, setState: vi.fn(), subscribe: vi.fn() }),
}));

import { tileStartVolume, usemultiNookStore } from './multiNookStore';

const store = () => usemultiNookStore.getState();

function startVolume(volume: unknown) {
  app.state.settings = volume === undefined ? {} : { video_player: { volume } };
}

beforeEach(() => {
  startVolume(0.09);
  usemultiNookStore.setState({ slots: [], isMultiNookActive: false });
  vi.mocked(invoke).mockClear();
});

// Every tile used to open at a fixed 50%, whatever Start volume said.
describe('new tiles open at the Start volume', () => {
  it('reads the player Start volume', () => {
    expect(tileStartVolume()).toBe(0.09);
  });

  it('falls back to full volume, the setting default, when there is none', () => {
    startVolume(undefined);
    expect(tileStartVolume()).toBe(1);
  });

  it('never hands a tile an unusable volume', () => {
    startVolume(1.7);
    expect(tileStartVolume()).toBe(1);
    startVolume(Number.NaN);
    expect(tileStartVolume()).toBe(1);
    startVolume(-0.2);
    expect(tileStartVolume()).toBe(0);
  });

  it('applies to channels added one by one, first audible and the rest muted', async () => {
    await store().addSlot('a', 'kick');
    await store().addSlot('b', 'kick');
    const slots = store().slots;
    expect(slots.map((s) => s.volume)).toEqual([0.09, 0.09]);
    expect(slots.map((s) => s.muted)).toEqual([false, true]);
  });

  it('applies to a preset loaded in replace mode, which builds its tiles inline', async () => {
    await store().loadPresetChannels([{ channelLogin: 'a' }, { channelLogin: 'b' }], 'replace');
    const slots = store().slots;
    expect(slots.map((s) => s.volume)).toEqual([0.09, 0.09]);
    expect(slots.map((s) => s.muted)).toEqual([false, true]);
  });
});

// A tile's volume is written in place by Rust and never refreshed this window's
// copy of the saved grid, which is what re-entering MultiNook restores from.
describe('leaving MultiNook keeps the tile volumes', () => {
  it('saves the grid with the current volumes before clearing it', async () => {
    const tile: MultiNookSlot = { id: 'cell-a', channelLogin: 'a', volume: 0.09, muted: false, isFocused: true };
    usemultiNookStore.setState({ slots: [tile], isMultiNookActive: true });
    store().updateSlot('cell-a', { volume: 0.4 });

    await store().toggleMultiNook();

    const save = vi.mocked(invoke).mock.calls.find(
      ([cmd, args]) => cmd === 'patch_settings' && Array.isArray((args as { patch?: { multi_nook_slots?: unknown } })?.patch?.multi_nook_slots),
    );
    const saved = (save?.[1] as { patch: { multi_nook_slots: MultiNookSlot[] } }).patch.multi_nook_slots;
    expect(saved[0].volume).toBe(0.4);
    expect(store().slots).toEqual([]);
  });

  it('does not overwrite a saved grid when nothing is open', async () => {
    usemultiNookStore.setState({ slots: [], isMultiNookActive: true });
    await store().toggleMultiNook();
    expect(vi.mocked(invoke).mock.calls.some(([cmd]) => cmd === 'patch_settings')).toBe(false);
  });
});
