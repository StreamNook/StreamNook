import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { learnedLLCushion, llCushionKey, rememberLLCushion } from './llCushion';

const DAY = 86_400_000;
const NOW = 1_800_000_000_000;

let store: Map<string, string>;

beforeEach(() => {
  store = new Map();
  vi.stubGlobal('localStorage', {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
  });
  vi.spyOn(Date, 'now').mockReturnValue(NOW);
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const save = (channel: string, c: number, t = NOW) =>
  store.set(llCushionKey(channel), JSON.stringify({ c, t }));

describe('learned live-edge cushion', () => {
  it('is the base until a channel has stalled', () => {
    expect(learnedLLCushion('kathi', 1.5)).toBe(1.5);
    expect(learnedLLCushion(undefined, 1.5)).toBe(1.5);
  });

  it('starts a stalled channel where it settled, capped at the base plus 2 s', () => {
    save('kathi', 3.0);
    expect(learnedLLCushion('kathi', 1.5)).toBe(3.0);
    save('kathi', 9.0);
    expect(learnedLLCushion('kathi', 1.5)).toBe(3.5);
  });

  it('relaxes half a second a day and never goes under the base', () => {
    save('kathi', 3.5, NOW - 2 * DAY);
    expect(learnedLLCushion('kathi', 1.5)).toBe(2.5);
    save('kathi', 1.0);
    expect(learnedLLCushion('kathi', 1.5)).toBe(1.5);
  });

  it('falls back to the base when storage fails', () => {
    vi.stubGlobal('localStorage', {
      getItem: () => {
        throw new Error('blocked');
      },
    });
    expect(learnedLLCushion('kathi', 1.5)).toBe(1.5);
  });

  it('remembers per platform and lowercased login', () => {
    rememberLLCushion('Kathi', 2.0);
    expect(JSON.parse(store.get('streamnook.ll-cushion.v4.twitch.kathi') ?? '{}')).toEqual({ c: 2.0, t: NOW });
  });
});
