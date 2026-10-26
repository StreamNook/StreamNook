import { afterEach, describe, expect, it, vi } from 'vitest';
import { observeOffscreen, resetOffscreenObserver } from './offscreenPause';

type Entry = { target: unknown; isIntersecting: boolean };

afterEach(() => {
  resetOffscreenObserver();
  vi.unstubAllGlobals();
});

describe('offscreenPause', () => {
  it('flags and clears data-sn-off', () => {
    let fire: ((entries: Entry[]) => void) | null = null;
    const observed = new Set<unknown>();
    class FakeObserver {
      constructor(cb: (entries: Entry[]) => void) {
        fire = cb;
      }
      observe(el: unknown) {
        observed.add(el);
      }
      unobserve(el: unknown) {
        observed.delete(el);
      }
      disconnect() {
        observed.clear();
      }
    }
    vi.stubGlobal('IntersectionObserver', FakeObserver);

    const el = { dataset: {} as Record<string, string> };
    const stop = observeOffscreen(el as unknown as HTMLElement);
    expect(observed.has(el)).toBe(true);

    fire!([{ target: el, isIntersecting: false }]);
    expect(el.dataset.snOff).toBe('true');
    fire!([{ target: el, isIntersecting: true }]);
    expect(el.dataset.snOff).toBeUndefined();

    fire!([{ target: el, isIntersecting: false }]);
    stop();
    expect(observed.has(el)).toBe(false);
    expect(el.dataset.snOff).toBeUndefined();
  });

  it('no IntersectionObserver gives a no-op', () => {
    vi.stubGlobal('IntersectionObserver', undefined);
    const el = { dataset: {} as Record<string, string> };
    const stop = observeOffscreen(el as unknown as HTMLElement);
    stop();
    expect(el.dataset.snOff).toBeUndefined();
  });
});
