// SN-284: the window's pin surface map and its one debounced call.
import { describe, expect, it, vi } from 'vitest';
import { arrivalPinnedOnScreen, createPinSurfaceRegistry, type PinSurfaceCall, type PinSurfaceSpec } from './pinSurfaces';

const spec = (id: string, over: Partial<PinSurfaceSpec> = {}): PinSurfaceSpec => ({
  id,
  rooms: [{ provider: 'twitch', channel: 'xqc' }],
  active: true,
  shown: true,
  mode: '',
  strips_only: false,
  over_video: false,
  opens: ['twitch'],
  visible: [],
  ...over,
});

function setup(seqStart?: number) {
  const calls: PinSurfaceCall[] = [];
  const pending: Array<() => void> = [];
  let stored = seqStart;
  const reg = createPinSurfaceRegistry({
    page: 'page-1',
    send: (c) => {
      calls.push(c);
    },
    seqStore: { get: () => stored, set: (n) => (stored = n) },
    schedule: (fn) => {
      pending.push(fn);
      return pending.length - 1;
    },
    cancel: (h) => {
      pending[h as number] = () => {};
    },
  });
  const tick = () => {
    const fns = pending.splice(0);
    fns.forEach((f) => f());
  };
  return { reg, calls, tick, stored: () => stored };
}

describe('pin surface registry', () => {
  it('sends nothing while Pin mentions is off', () => {
    const { reg, calls, tick } = setup();
    reg.register('t1', spec('main-chat'));
    reg.update('t1', { shown: false });
    tick();
    expect(calls).toEqual([]);
  });

  it('sends the whole map at once when turned on, as seq 0 of a new page', () => {
    const { reg, calls } = setup();
    reg.register('t1', spec('main-chat'));
    reg.setEnabled(true);
    expect(calls).toHaveLength(1);
    expect(calls[0].page).toBe('page-1');
    expect(calls[0].seq).toBe(0);
    expect(calls[0].surfaces.map((s) => s.id)).toEqual(['main-chat']);
    expect(calls[0].surfaces[0].token).toBe('t1');
  });

  it('keeps seq going across a hot reload (the store holds the last one)', () => {
    const { reg, calls } = setup(41);
    reg.setEnabled(true);
    expect(calls[0].seq).toBe(42);
  });

  it('coalesces a burst into one debounced call, newest state', () => {
    const { reg, calls, tick } = setup();
    reg.setEnabled(true);
    calls.length = 0;
    reg.register('t1', spec('main-chat'));
    reg.register('t2', spec('dock'));
    reg.update('t1', { active: false });
    tick();
    expect(calls).toHaveLength(1);
    expect(calls[0].surfaces.map((s) => [s.id, s.active])).toEqual([
      ['main-chat', false],
      ['dock', true],
    ]);
  });

  it('survives a StrictMode double mount: an unmount removes only its own token', () => {
    const { reg, calls, tick } = setup();
    reg.setEnabled(true);
    calls.length = 0;
    // mount, unmount, mount again (the same id, new mount)
    reg.register('a', spec('main-chat'));
    reg.unregister('a');
    reg.register('b', spec('main-chat'));
    // the first mount's late cleanup names its own token, which is gone
    reg.unregister('a');
    tick();
    expect(reg.peek().map((s) => s.token)).toEqual(['b']);
    expect(calls.at(-1)?.surfaces.map((s) => s.token)).toEqual(['b']);
  });

  it('reports no visible pins for a surface that is not shown', () => {
    const { reg } = setup();
    reg.setEnabled(true);
    reg.register('t1', spec('main-chat', { shown: false, visible: ['a', 'b'] }));
    reg.register('t2', spec('dock', { shown: true, visible: ['c'] }));
    const out = reg.peek();
    expect(out.find((s) => s.id === 'main-chat')?.visible).toEqual([]);
    expect(out.find((s) => s.id === 'dock')?.visible).toEqual(['c']);
  });

  it('does not send for an update that changes nothing, and does for a resend', () => {
    const { reg, calls, tick } = setup();
    reg.setEnabled(true);
    reg.register('t1', spec('main-chat'));
    tick();
    calls.length = 0;
    reg.update('t1', { active: true });
    tick();
    expect(calls).toHaveLength(0);
    reg.resend();
    tick();
    expect(calls).toHaveLength(1);
  });

  it('stops sending when turned off, and sends the map again when turned back on', () => {
    const { reg, calls, tick } = setup();
    reg.setEnabled(true);
    reg.register('t1', spec('main-chat'));
    reg.setEnabled(false);
    calls.length = 0;
    reg.update('t1', { shown: false });
    tick();
    expect(calls).toEqual([]);
    reg.setEnabled(true);
    expect(calls).toHaveLength(1);
    expect(calls[0].surfaces[0].shown).toBe(false);
  });

  it('logs a failed send without throwing', async () => {
    const send = vi.fn(() => Promise.reject(new Error('refused')));
    const reg = createPinSurfaceRegistry({ page: 'p', send });
    expect(() => reg.setEnabled(true)).not.toThrow();
    await Promise.resolve();
    expect(send).toHaveBeenCalledTimes(1);
  });
});

describe('arrivalPinnedOnScreen', () => {
  const surfaces = [
    { id: 'phone-watch', shown: true },
    { id: 'phone-chats', shown: false },
  ];
  it('skips the card for a mention pinned on a surface that is in front', () => {
    expect(arrivalPinnedOnScreen('phone-watch', surfaces)).toBe(true);
  });
  it('shows the card when the pin surface is not in front, unknown, or there is none', () => {
    expect(arrivalPinnedOnScreen('phone-chats', surfaces)).toBe(false);
    expect(arrivalPinnedOnScreen('dock', surfaces)).toBe(false);
    expect(arrivalPinnedOnScreen(null, surfaces)).toBe(false);
    expect(arrivalPinnedOnScreen(undefined, surfaces)).toBe(false);
  });
});
