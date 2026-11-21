// @vitest-environment happy-dom
// SN-284: the pins block. X closes one through Rust, Clear all closes the ids
// that were drawn when the press began, a press opens by location, and while
// the pointer moved here a pin Rust ended stays drawn (D16).
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { MotionGlobalConfig } from 'framer-motion';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

MotionGlobalConfig.skipAnimations = true;
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => null), convertFileSrc: (p: string) => p, Channel: class {} }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
}));
vi.mock('../../utils/platform', () => ({
  IS_MOBILE: false,
  IS_ANDROID: false,
  IS_MAC: false,
  IS_LINUX: false,
  IS_WEBKITGTK: false,
  IS_MAIN_WINDOW: true,
  isMainWindow: () => true,
}));

import { invoke } from '@tauri-apps/api/core';
import { MentionPinsBlock } from './MentionPins';
import { samplePin } from '../settings/pinSamples';
import type { MentionPin } from '../../types';

let host: HTMLElement;
let root: Root;

const pins = (...ns: number[]): MentionPin[] =>
  ns.map((n) => samplePin(n, 'mention', `user${n}`, '#ffffff', `message ${n}`, '9:00 PM'));

function render(list: MentionPin[], over: Partial<React.ComponentProps<typeof MentionPinsBlock>> = {}) {
  act(() =>
    root.render(
      <MentionPinsBlock
        pins={list}
        form="full"
        shown={false}
        warm={false}
        onOpenOwn={over.onOpenOwn ?? (() => {})}
        onOpenOther={over.onOpenOther ?? (() => {})}
        {...over}
      />,
    ),
  );
}

const dismissCalls = () =>
  vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === 'dismiss_mention_pins').map(([, args]) => (args as { ids: string[] }).ids);

beforeEach(() => {
  vi.mocked(invoke).mockClear();
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.useRealTimers();
});

describe('MentionPinsBlock', () => {
  it('draws nothing with no pins', () => {
    render([]);
    expect(host.querySelector('[data-mention-pins]')).toBeNull();
  });

  it('draws the oldest first and the newest last', () => {
    render(pins(1, 2, 3));
    const ids = [...host.querySelectorAll('[data-pin-id]')].map((e) => (e as HTMLElement).dataset.pinId);
    expect(ids).toEqual(['sample-pin-1', 'sample-pin-2', 'sample-pin-3']);
  });

  it('the X closes one pin through Rust', () => {
    render(pins(1, 2));
    const x = host.querySelector('[data-pin-id="sample-pin-1"] button[aria-label="Dismiss this pinned mention"]') as HTMLElement;
    act(() => x.click());
    expect(dismissCalls()).toEqual([['sample-pin-1']]);
    expect(host.querySelector('[data-pin-id="sample-pin-1"]')).toBeNull();
  });

  it('Clear all closes the ids drawn when the press began, not a pin that landed after', () => {
    render(pins(1, 2));
    const clear = [...host.querySelectorAll('button')].find((b) => b.textContent === 'Clear all') as HTMLElement;
    act(() => {
      clear.dispatchEvent(new Event('pointerdown', { bubbles: true, cancelable: true }));
    });
    // A third pin lands between the press and the click.
    render(pins(1, 2, 3));
    const clear2 = [...host.querySelectorAll('button')].find((b) => b.textContent === 'Clear all') as HTMLElement;
    act(() => clear2.click());
    expect(dismissCalls()).toEqual([['sample-pin-1', 'sample-pin-2']]);
  });

  it('Clear all by keyboard (no press) closes what is drawn now, and a spent press is not reused', () => {
    render(pins(1, 2));
    const find = () => [...host.querySelectorAll('button')].find((b) => b.textContent === 'Clear all') as HTMLElement;
    // A press and click, then more pins land and the keyboard clears: the old press is spent.
    act(() => {
      find().dispatchEvent(new Event('pointerdown', { bubbles: true, cancelable: true }));
    });
    act(() => find().click());
    expect(dismissCalls().at(-1)).toEqual(['sample-pin-1', 'sample-pin-2']);
    render(pins(3, 4));
    act(() => find().click());
    expect(dismissCalls().at(-1)).toEqual(['sample-pin-3', 'sample-pin-4']);
  });

  it('opens a pin of its own chat with onOpenOwn and another chat with onOpenOther', () => {
    const own = vi.fn();
    const other = vi.fn();
    const list = pins(1, 2);
    list[1] = { ...list[1], channel_mark: { room: 'kick:x', platform: 'kick', channel: 'x' } };
    render(list, { onOpenOwn: own, onOpenOther: other });
    act(() => (host.querySelector('[data-pin-id="sample-pin-1"] [role="button"]') as HTMLElement).click());
    expect(own).toHaveBeenCalledTimes(1);
    act(() => (host.querySelector('[data-pin-id="sample-pin-2"] [role="button"]') as HTMLElement).click());
    expect(other).toHaveBeenCalledTimes(1);
  });

  it('holds a pin Rust ended while the pointer moved here, and lets go after 2 seconds', () => {
    vi.useFakeTimers();
    render(pins(1, 2, 3));
    const block = host.querySelector('[data-mention-pins]') as HTMLElement;
    act(() => {
      block.dispatchEvent(new MouseEvent('pointermove', { bubbles: true }));
    });
    render(pins(2, 3));
    expect(host.querySelector('[data-pin-id="sample-pin-1"]')).not.toBeNull();
    act(() => {
      vi.advanceTimersByTime(2100);
    });
    expect(host.querySelector('[data-pin-id="sample-pin-1"]')).toBeNull();
  });

  it('reports the pins in view only while shown', () => {
    const reportVisible = vi.fn();
    render(pins(1), { shown: false, reportVisible });
    expect(reportVisible).toHaveBeenLastCalledWith([]);
  });
});
