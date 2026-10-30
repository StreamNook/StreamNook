import { beforeEach, describe, expect, it, vi } from 'vitest';

// Mock factories are hoisted above the imports, so everything they use is
// created in `vi.hoisted`. Each factory provides every name the feed imports.
const h = vi.hoisted(() => {
  type Listener = () => void;
  const store = <S extends object>(initial: S) => {
    let state = initial;
    const listeners = new Set<Listener>();
    return {
      getState: () => state,
      setState: (patch: Partial<S>) => {
        state = { ...state, ...patch };
        listeners.forEach((l) => l());
      },
      subscribe: (l: Listener) => {
        listeners.add(l);
        return () => {
          listeners.delete(l);
        };
      },
    };
  };
  const calls: string[] = [];
  const invoke = vi.fn(async (cmd: string, args?: unknown) => {
    void args;
    calls.push(cmd);
    if (cmd === 'discord_presence_page') return 7;
    return 'applied';
  });
  const win = {
    label: 'main',
    pageHandler: null as (() => void) | null,
    listen: vi.fn(async (event: string, handler: () => void) => {
      calls.push(`listen:${event}`);
      win.pageHandler = handler;
      return () => {};
    }),
  };
  const grid = store({
    isMultiNookActive: true,
    maximizedSlotId: null as string | null,
    activeChatChannelId: null as string | null,
    slots: [{ id: 'a', channelLogin: 'a', volume: 1, muted: false, isFocused: true }],
  });
  const app = store({
    currentMediaType: null as string | null,
    streamUrl: null as string | null,
    vodPlayback: null as unknown,
    originalMediaUrl: null as string | null,
    currentStream: null,
  });
  return { calls, invoke, win, grid, app };
});

vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: unknown) => h.invoke(cmd, args) }));
vi.mock('@tauri-apps/api/webviewWindow', () => ({
  getCurrentWebviewWindow: () => ({ label: h.win.label, listen: h.win.listen }),
}));
vi.mock('./multiNookStore', () => ({
  usemultiNookStore: h.grid,
  activeChatSlot: () => null,
}));
vi.mock('./AppStore', () => ({ useAppStore: h.app }));

import { installDiscordPresenceFeed } from './discordPresenceFeed';

const settle = () => new Promise((r) => setTimeout(r, 0));
const sends = () => h.calls.filter((c) => c.startsWith('discord_presence_set_'));

beforeEach(() => {
  h.calls.length = 0;
  h.invoke.mockClear();
  h.win.pageHandler = null;
  h.win.label = 'main';
});

describe('discord presence feed', () => {
  it('does nothing outside the main window', async () => {
    h.win.label = 'multichat-1';
    const stop = await installDiscordPresenceFeed();
    await settle();
    expect(h.calls).toEqual([]);
    stop();
  });

  it('listens for the page event before reading the generation, then sends both values', async () => {
    const stop = await installDiscordPresenceFeed();
    await settle();
    expect(h.calls.slice(0, 2)).toEqual(['listen:discord-presence://page', 'discord_presence_page']);
    expect(sends().sort()).toEqual(['discord_presence_set_grid', 'discord_presence_set_media']);
    const gridCall = h.invoke.mock.calls.find((c) => c[0] === 'discord_presence_set_grid');
    expect((gridCall?.[1] as { gen: number }).gen).toBe(7);
    stop();
  });

  it('a volume drag sends nothing', async () => {
    const stop = await installDiscordPresenceFeed();
    await settle();
    h.calls.length = 0;
    h.grid.setState({ slots: [{ id: 'a', channelLogin: 'a', volume: 0.3, muted: true, isFocused: true }] });
    await settle();
    expect(sends()).toEqual([]);
    stop();
  });

  it('the page event reads the generation again and resends both values', async () => {
    const stop = await installDiscordPresenceFeed();
    await settle();
    h.calls.length = 0;
    h.win.pageHandler?.();
    await settle();
    await settle();
    expect(h.calls[0]).toBe('discord_presence_page');
    expect(sends().sort()).toEqual(['discord_presence_set_grid', 'discord_presence_set_media']);
    stop();
  });

  it('a failed generation read does not stop the feed', async () => {
    // The first invoke is the generation read (listening is not an invoke).
    h.invoke.mockImplementationOnce(async () => {
      throw new Error('ipc');
    });
    const stop = await installDiscordPresenceFeed();
    await settle();
    expect(sends().length).toBeGreaterThan(0);
    stop();
  });
});
