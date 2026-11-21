// @vitest-environment happy-dom
// SN-284: the pins hook asks Rust only while Pin mentions is on, draws only the
// latest request's answer, clears at once when its surface changes, and
// registers the surface it is mounted as (one token per mount).
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

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
import { pinSurfaces } from '../utils/pinSurfaces';
import { samePins, useMentionPins, usePinSurface } from './useMentionPins';
import { samplePin } from '../components/settings/pinSamples';
import type { MentionPin, MentionPins } from '../types';

const pin = (n: number, over: Partial<MentionPin> = {}): MentionPin => ({
  ...samplePin(n, 'mention', `u${n}`, '#fff', `m${n}`, '9:00 PM'),
  ...over,
});
const answer = (pins: MentionPin[], revision = 1): MentionPins => ({ revision, pins, max: 10 });

describe('samePins', () => {
  it('is false when an id, a row revision, a clock state or a copy count differs', () => {
    expect(samePins([pin(1)], [pin(1)])).toBe(true);
    expect(samePins([pin(1)], [pin(2)])).toBe(false);
    expect(samePins([pin(1)], [pin(1, { rev: 99 })])).toBe(false);
    expect(samePins([pin(1)], [pin(1, { clock: 'none' })])).toBe(false);
    expect(samePins([pin(1)], [pin(1, { run: 2 })])).toBe(false);
    expect(samePins([pin(1)], [])).toBe(false);
  });
});

let host: HTMLElement;
let root: Root;
const out: { seen: MentionPin[] } = { seen: [] };

function Probe({ surface }: { surface: string | null }) {
  out.seen = useMentionPins(surface).pins;
  return null;
}

function Surface({ id }: { id: string }) {
  usePinSurface({
    id,
    rooms: [{ provider: 'twitch', channel: 'xqc' }],
    active: true,
    shown: true,
    mode: '',
    strips_only: false,
    over_video: false,
    opens: ['twitch'],
  });
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
    act(() => root.render(<Probe surface="main-chat" />));
    await flush();
    expect(vi.mocked(invoke).mock.calls.some(([cmd]) => cmd === 'get_mention_pins')).toBe(false);
    expect(out.seen).toEqual([]);
  });

  it('asks for its surface by page and id', async () => {
    setPinSetting(true);
    vi.mocked(invoke).mockResolvedValue(answer([pin(1)]));
    act(() => root.render(<Probe surface="dock" />));
    await flush();
    expect(vi.mocked(invoke)).toHaveBeenCalledWith('get_mention_pins', {
      page: String(performance.timeOrigin),
      surface: 'dock',
    });
    expect(out.seen.map((p) => p.entry.id)).toEqual(['sample-pin-1']);
  });

  it('draws only the latest answer, and clears when the surface changes', async () => {
    setPinSetting(true);
    const answers: Array<(v: MentionPins) => void> = [];
    vi.mocked(invoke).mockImplementation(() => new Promise((resolve) => answers.push(resolve as never)));
    act(() => root.render(<Probe surface="main-chat" />));
    await flush();
    act(() => root.render(<Probe surface="dock" />));
    await flush();
    // The first surface's slow answer lands after the second was asked: dropped.
    await act(async () => answers[0](answer([pin(1)])));
    expect(out.seen).toEqual([]);
    await act(async () => answers[answers.length - 1](answer([pin(2)])));
    expect(out.seen.map((p) => p.entry.id)).toEqual(['sample-pin-2']);
    act(() => root.render(<Probe surface="main-chat" />));
    expect(out.seen).toEqual([]);
  });
});

describe('usePinSurface', () => {
  it('registers once per mount, with its own token, and removes only its own on unmount', () => {
    pinSurfaces.setEnabled(false);
    const before = pinSurfaces.size();
    act(() => root.render(<Surface id="main-chat" />));
    expect(pinSurfaces.size()).toBe(before + 1);
    const tokens = pinSurfaces.peek().map((s) => s.token);
    act(() => root.render(<Surface id="main-chat" />));
    expect(pinSurfaces.peek().map((s) => s.token)).toEqual(tokens);
    act(() => root.unmount());
    expect(pinSurfaces.size()).toBe(before);
    root = createRoot(host);
  });
});
