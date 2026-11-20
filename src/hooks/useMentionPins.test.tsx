// @vitest-environment happy-dom
// SN-284: the pins hook asks Rust only while Pin mentions is on, draws only
// the latest request's answer, clears at once when the chat's rooms change,
// and asks again once, a little after the earliest pin's end.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

// The store asks for settings as it loads: answer nothing.
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => null) }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
}));
vi.mock('../utils/platform', () => ({
  IS_MOBILE: false,
  IS_ANDROID: false,
  IS_MAC: false,
  IS_LINUX: false,
  IS_WEBKITGTK: false,
  IS_MAIN_WINDOW: true,
  isMainWindow: () => true,
}));

import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../stores/AppStore';
import { nextRefreshDelay, samePins, useMentionPins } from './useMentionPins';
import type { MentionPin, MentionPins } from '../types';

const pin = (id: string, until: number, run = 1): MentionPin => ({
  entry: {
    id,
    platform: 'twitch',
    room: 'xqc',
    channel_login: 'xqc',
    sender_id: '1',
    sender_login: 'a',
    sender_name: 'A',
    text: '@you',
    kind: 'mention',
    at_ms: 0,
    read: true,
    removed: false,
  },
  until_ms: until,
  run,
});

describe('nextRefreshDelay', () => {
  it('waits until just past the earliest end, on the clock Rust answered with', () => {
    expect(nextRefreshDelay([pin('a', 9_000), pin('b', 4_000)], 1_000)).toBe(3_050);
    expect(nextRefreshDelay([pin('a', 500)], 1_000)).toBe(50);
    expect(nextRefreshDelay([], 1_000)).toBeNull();
  });
});

describe('samePins', () => {
  it('is false when an id, an end or a copy count differs', () => {
    expect(samePins([pin('a', 1)], [pin('a', 1)])).toBe(true);
    expect(samePins([pin('a', 1)], [pin('b', 1)])).toBe(false);
    expect(samePins([pin('a', 1)], [pin('a', 2)])).toBe(false);
    expect(samePins([pin('a', 1, 1)], [pin('a', 1, 2)])).toBe(false);
    expect(samePins([pin('a', 1)], [])).toBe(false);
  });
});

let host: HTMLElement;
let root: Root;
const out: { seen: MentionPin[] } = { seen: [] };
const report = (pins: MentionPin[]) => {
  out.seen = pins;
};

function Probe({ rooms }: { rooms: string }) {
  report(useMentionPins(rooms));
  return null;
}

const setPinSetting = (on: boolean) =>
  useAppStore.setState((s) => ({
    settings: { ...s.settings, chat_design: { ...(s.settings.chat_design ?? {}), pin_mentions: on } } as never,
  }));

const flush = async () => {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
};

beforeEach(() => {
  vi.mocked(invoke).mockReset();
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  out.seen = [];
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.useRealTimers();
});

describe('useMentionPins', () => {
  it('asks nothing while Pin mentions is off', async () => {
    setPinSetting(false);
    act(() => root.render(<Probe rooms="xqc" />));
    await flush();
    expect(vi.mocked(invoke).mock.calls.some(([cmd]) => cmd === 'get_mention_pins')).toBe(false);
    expect(out.seen).toEqual([]);
  });

  it('draws only the latest answer, and clears when the rooms change', async () => {
    setPinSetting(true);
    const answers: Array<(v: MentionPins) => void> = [];
    vi.mocked(invoke).mockImplementation(() => new Promise((resolve) => answers.push(resolve as never)));
    act(() => root.render(<Probe rooms="xqc" />));
    act(() => root.render(<Probe rooms="kick:other" />));
    await flush();
    expect(answers.length).toBe(2);
    // The second room's answer lands first, then the first room's, late.
    await act(async () => answers[1]({ pins: [pin('new', 99_000)], now_ms: 1_000 }));
    await act(async () => answers[0]({ pins: [pin('old', 99_000)], now_ms: 1_000 }));
    expect(out.seen.map((p) => p.entry.id)).toEqual(['new']);
    // Moving to a room with no answer yet shows nothing from the last one.
    vi.mocked(invoke).mockImplementation(() => new Promise(() => {}));
    act(() => root.render(<Probe rooms="other" />));
    expect(out.seen).toEqual([]);
  });

  it('draws the latest ask for the same rooms when an older answer lands last', async () => {
    setPinSetting(true);
    const answers: Array<(v: MentionPins) => void> = [];
    vi.mocked(invoke).mockImplementation(
      (cmd) => (cmd === 'get_mention_pins' ? new Promise((resolve) => answers.push(resolve as never)) : Promise.resolve(null)) as never,
    );
    act(() => root.render(<Probe rooms="xqc" />));
    await flush();
    // The window comes back: a second ask for the same rooms.
    act(() => {
      window.dispatchEvent(new Event('focus'));
    });
    await flush();
    expect(answers.length).toBe(2);
    await act(async () => answers[1]({ pins: [pin('fresh', 99_000)], now_ms: 1_000 }));
    await act(async () => answers[0]({ pins: [pin('dismissed', 99_000)], now_ms: 1_000 }));
    expect(out.seen.map((p) => p.entry.id)).toEqual(['fresh']);
  });

  it('asks again once, just past the earliest end', async () => {
    vi.useFakeTimers();
    setPinSetting(true);
    vi.mocked(invoke)
      .mockResolvedValueOnce({ pins: [pin('a', 6_000), pin('b', 3_000)], now_ms: 1_000 } satisfies MentionPins)
      .mockResolvedValue({ pins: [], now_ms: 3_100 } satisfies MentionPins);
    act(() => root.render(<Probe rooms="xqc" />));
    await flush();
    expect(out.seen.length).toBe(2);
    expect(invoke).toHaveBeenCalledTimes(1);
    await act(async () => {
      vi.advanceTimersByTime(2_049);
    });
    expect(invoke).toHaveBeenCalledTimes(1);
    await act(async () => {
      vi.advanceTimersByTime(2);
    });
    await flush();
    expect(invoke).toHaveBeenCalledTimes(2);
    expect(out.seen).toEqual([]);
  });
});
