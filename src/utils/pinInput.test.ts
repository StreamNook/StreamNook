// @vitest-environment happy-dom
// SN-284: the viewer-input report is leading and trailing edge, at most once
// per second, and the window listeners are installed only while pins are on.
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => null) }));

import { invoke } from '@tauri-apps/api/core';
import { createInputNotifier, INPUT_EVENTS, setPinInputEnabled } from './pinInput';

function clock() {
  let t = 10_000;
  const pending: Array<{ at: number; fn: () => void; live: boolean }> = [];
  return {
    now: () => t,
    schedule: (fn: () => void, ms: number) => {
      const h = { at: t + ms, fn, live: true };
      pending.push(h);
      return h;
    },
    cancel: (h: unknown) => {
      (h as { live: boolean }).live = false;
    },
    advance(ms: number) {
      t += ms;
      for (const h of pending) {
        if (h.live && h.at <= t) {
          h.live = false;
          h.fn();
        }
      }
    },
  };
}

describe('createInputNotifier', () => {
  it('sends on the leading edge, then once on the trailing edge, at most once a second', () => {
    const c = clock();
    const send = vi.fn();
    const n = createInputNotifier({ send, now: c.now, schedule: c.schedule, cancel: c.cancel });
    n.touch(); // leading
    expect(send).toHaveBeenCalledTimes(1);
    for (let i = 0; i < 50; i++) {
      c.advance(10);
      n.touch();
    }
    expect(send).toHaveBeenCalledTimes(1);
    c.advance(500); // 1000 ms since the leading send: the trailing one fires
    expect(send).toHaveBeenCalledTimes(2);
    c.advance(5000);
    expect(send).toHaveBeenCalledTimes(2);
    n.touch(); // a new burst leads again
    expect(send).toHaveBeenCalledTimes(3);
  });

  it('dispose drops a queued trailing send', () => {
    const c = clock();
    const send = vi.fn();
    const n = createInputNotifier({ send, now: c.now, schedule: c.schedule, cancel: c.cancel });
    n.touch();
    n.touch();
    n.dispose();
    c.advance(2000);
    expect(send).toHaveBeenCalledTimes(1);
  });
});

describe('setPinInputEnabled', () => {
  afterEach(() => setPinInputEnabled(false));

  it('installs capture-phase passive listeners only while on, and reports input', () => {
    vi.mocked(invoke).mockClear();
    const add = vi.spyOn(window, 'addEventListener');
    const remove = vi.spyOn(window, 'removeEventListener');
    setPinInputEnabled(true);
    for (const name of INPUT_EVENTS) {
      expect(add).toHaveBeenCalledWith(name, expect.any(Function), { capture: true, passive: true });
    }
    window.dispatchEvent(new Event('pointerdown'));
    expect(vi.mocked(invoke)).toHaveBeenCalledWith('note_viewer_input');
    setPinInputEnabled(false);
    for (const name of INPUT_EVENTS) {
      expect(remove).toHaveBeenCalledWith(name, expect.any(Function), { capture: true });
    }
    vi.mocked(invoke).mockClear();
    window.dispatchEvent(new Event('pointerdown'));
    expect(vi.mocked(invoke)).not.toHaveBeenCalled();
    add.mockRestore();
    remove.mockRestore();
  });
});
