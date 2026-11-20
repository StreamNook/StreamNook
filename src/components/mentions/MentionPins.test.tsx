// @vitest-environment happy-dom
// SN-284: the pins block holds still under the pointer, and putting away the
// last line it holds lets the hold go (the block unmounts, so no pointerleave
// ever comes): the next pin still shows.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { MotionGlobalConfig } from 'framer-motion';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

MotionGlobalConfig.skipAnimations = true;
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => null) }));
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
import { MentionPinsView } from './MentionPins';
import type { MentionPin } from '../../types';

const pin = (id: string, text: string): MentionPin => ({
  entry: {
    id,
    platform: 'twitch',
    room: 'xqc',
    channel_login: 'xqc',
    sender_id: id,
    sender_login: id,
    sender_name: id,
    text,
    kind: 'mention',
    at_ms: 0,
    read: true,
    removed: false,
  },
  until_ms: 0,
  run: 1,
});

let host: HTMLElement;
let root: Root;

beforeEach(() => {
  vi.mocked(invoke).mockClear();
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const block = () => host.querySelector('[data-mention-pin]')?.parentElement?.parentElement as HTMLElement;
const ids = () => [...host.querySelectorAll('[data-mention-pin]')].map((el) => el.getAttribute('data-mention-pin'));

describe('MentionPinsView', () => {
  it('lets the hold go when the last line it holds is put away', async () => {
    const onOpen = vi.fn();
    act(() => root.render(<MentionPinsView live={[pin('a', '@you one')]} onOpen={onOpen} />));
    expect(ids()).toEqual(['a']);
    act(() => {
      block().dispatchEvent(new PointerEvent('pointerover', { bubbles: true }));
      block().dispatchEvent(new PointerEvent('pointerenter', { bubbles: false }));
    });
    const x = host.querySelector('button[aria-label="Dismiss this pinned mention"]') as HTMLButtonElement;
    await act(async () => x.click());
    expect(invoke).toHaveBeenCalledWith('dismiss_mention_pin', { id: 'a' });
    expect(ids()).toEqual([]);
    // Rust's next answer: a new mention. It shows even though the pointer
    // never "left" the block that went away.
    act(() => root.render(<MentionPinsView live={[pin('b', '@you two')]} onOpen={onOpen} />));
    expect(ids()).toEqual(['b']);
  });

  it('holds still under the pointer, then shows the newest on leave', () => {
    const onOpen = vi.fn();
    act(() => root.render(<MentionPinsView live={[pin('a', '@you one')]} onOpen={onOpen} />));
    act(() => {
      block().dispatchEvent(new PointerEvent('pointerover', { bubbles: true }));
      block().dispatchEvent(new PointerEvent('pointerenter', { bubbles: false }));
    });
    act(() => root.render(<MentionPinsView live={[pin('b', '@you two'), pin('a', '@you one')]} onOpen={onOpen} />));
    expect(ids()).toEqual(['a']);
    act(() => {
      block().dispatchEvent(new PointerEvent('pointerout', { bubbles: true }));
      block().dispatchEvent(new PointerEvent('pointerleave', { bubbles: false }));
    });
    expect(ids()).toEqual(['b']);
  });

  it('opens with the line, not with its X', async () => {
    const onOpen = vi.fn();
    act(() => root.render(<MentionPinsView live={[pin('a', '@you one')]} onOpen={onOpen} />));
    const openButton = host.querySelector('button[aria-label$="Open it"]') as HTMLButtonElement;
    await act(async () => openButton.click());
    expect(onOpen).toHaveBeenCalledTimes(1);
    expect(invoke).not.toHaveBeenCalledWith('dismiss_mention_pin', expect.anything());
  });
});
