import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// How addToast treats an id that is already showing: the new toast replaces it
// in place, and a toast with a different id or none still appends.

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
}));
vi.mock('../bootPreload', () => ({ takePreloadedSettings: () => null }));

import { useAppStore } from './AppStore';

const add = (message: string, id?: number, extra: { sticky?: boolean } = {}) =>
  useAppStore.getState().addToast(message, 'info', undefined, { skipIsland: true, alwaysShow: true, id, ...extra });

const messages = () => useAppStore.getState().toasts.map((t) => t.message);

beforeEach(() => {
  vi.useFakeTimers();
  useAppStore.setState({ toasts: [] });
});

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
