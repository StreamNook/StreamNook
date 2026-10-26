import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// How addToast treats an id that is already showing: the new toast replaces it
// in place, and a toast with a different id or none still appends.

const { emit } = vi.hoisted(() => ({ emit: vi.fn(async (_event: string, _payload?: unknown) => {}) }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
  emit,
}));
vi.mock('../bootPreload', () => ({ takePreloadedSettings: () => null }));

import { useAppStore } from './AppStore';

const add = (message: string, id?: number, extra: { sticky?: boolean } = {}) =>
  useAppStore.getState().addToast(message, 'info', undefined, { skipIsland: true, alwaysShow: true, id, ...extra });

const messages = () => useAppStore.getState().toasts.map((t) => t.message);

beforeEach(() => {
  vi.useFakeTimers();
  useAppStore.setState({ toasts: [] });
  emit.mockClear();
});

/** The same add, but with the island mirror left on. */
const addWithIsland = (message: string, id?: number, title?: string) =>
  useAppStore.getState().addToast(message, 'info', undefined, { alwaysShow: true, sticky: true, id, title });

afterEach(() => vi.useRealTimers());

describe('addToast with an id', () => {
  it('leaves one toast, with the second message, when the id is already showing', () => {
    add('first', -1, { sticky: true });
    add('second', -1, { sticky: true });
    expect(messages()).toEqual(['second']);
  });

  it('keeps the replaced toast in place and keeps its createdAt', () => {
    add('before', 7);
    add('target', -1, { sticky: true });
    add('after', 8);
    const createdAt = useAppStore.getState().toasts[1].createdAt;
    vi.advanceTimersByTime(1000);
    add('target again', -1, { sticky: true });
    const toasts = useAppStore.getState().toasts;
    expect(toasts.map((t) => t.message)).toEqual(['before', 'target again', 'after']);
    expect(toasts[1].createdAt).toBe(createdAt);
  });

  it('still appends for different ids and for no id', () => {
    add('a', 1);
    add('b', 2);
    add('c');
    add('d');
    expect(messages()).toEqual(['a', 'b', 'c', 'd']);
  });

  it('clears the first timer of a timed toast that is replaced by id', () => {
    add('first', 5);
    vi.advanceTimersByTime(4000);
    add('second', 5);
    // The first toast's deadline passes; the replacement is still showing.
    vi.advanceTimersByTime(1500);
    expect(messages()).toEqual(['second']);
    // The replacement leaves at its own deadline.
    vi.advanceTimersByTime(4000);
    expect(messages()).toEqual([]);
  });

  it('clears the timer when a sticky toast replaces a timed one', () => {
    add('timed', 6);
    add('sticky', 6, { sticky: true });
    vi.advanceTimersByTime(10_000);
    expect(messages()).toEqual(['sticky']);
  });
});

describe('the notification center mirror', () => {
  it('emits once for two adds under the same id', () => {
    addWithIsland('first', -2);
    addWithIsland('second', -2);
    expect(emit).toHaveBeenCalledTimes(1);
    expect(messages()).toEqual(['second']);
  });

  it('emits twice for two different ids', () => {
    addWithIsland('first', -3);
    addWithIsland('second', -4);
    expect(emit).toHaveBeenCalledTimes(2);
  });

  it('emits again once the toast was removed', () => {
    addWithIsland('first', -5);
    useAppStore.getState().removeToast(-5);
    addWithIsland('again', -5);
    expect(emit).toHaveBeenCalledTimes(2);
  });

  it('puts a title before the message in the island text and keeps it on the toast', () => {
    addWithIsland('Restart to fix it.', -6, 'Video and chat are lagging');
    expect(emit).toHaveBeenCalledWith(
      'action-notification',
      expect.objectContaining({ text: 'Video and chat are lagging. Restart to fix it.' }),
    );
    expect(useAppStore.getState().toasts[0].title).toBe('Video and chat are lagging');
  });
});
