import { describe, it, expect } from 'vitest';
import { makeKey } from './providerKey';

/**
 * The chat-slice key contract, asserted against what STORAGE does.
 *
 * `chatConnectionStore.setSlice` lowercases unconditionally before storing, so
 * the only key that can ever be found in `channels` is the lowercase one. These
 * cases pin that, deliberately phrased around storage rather than around what a
 * caller intends, because the previous attempt at this asserted the caller's
 * intent (case-preserving, "because makeKey preserves case") and killed YouTube
 * chat outright on device.
 *
 * Duplicated here rather than imported: the real helper lives in the store, and
 * the store reaches AppStore/bootPreload, which touch `window` at module scope
 * and cannot be imported in this node test environment.
 */
function sliceLookupKey(provider: 'twitch' | 'kick' | 'youtube', channel: string): string {
  return (provider === 'twitch' ? channel : makeKey(provider, channel)).toLowerCase();
}

/** What setSlice does to whatever it is handed. */
const asStored = (k: string) => k.toLowerCase();

describe('slice lookup key matches what setSlice stores', () => {
  // The bug: makeKey preserves YouTube case, storage does not, so the composite
  // key missed its own slice. 0 hits in 40 lookups on a real session.
  it('folds a mixed-case YouTube id, which makeKey alone does NOT', () => {
    expect(makeKey('youtube', 'HVtwmO9RLNw')).toBe('youtube:HVtwmO9RLNw');
    expect(sliceLookupKey('youtube', 'HVtwmO9RLNw')).toBe('youtube:hvtwmo9rlnw');
  });

  it('round-trips through storage for every platform', () => {
    const cases: Array<['twitch' | 'kick' | 'youtube', string]> = [
      ['youtube', 'HVtwmO9RLNw'],
      ['youtube', 'tvXDeFwKzVM'],
      ['kick', 'SomeSlug'],
      ['twitch', 'XQc'],
      ['twitch', 'jynxzi'],
    ];
    for (const [provider, channel] of cases) {
      const lookup = sliceLookupKey(provider, channel);
      expect(lookup, `${provider}:${channel} must survive storage`).toBe(asStored(lookup));
    }
  });

  it('keeps Twitch bare, so legacy bare-login slices still resolve', () => {
    expect(sliceLookupKey('twitch', 'XQc')).toBe('xqc');
    expect(sliceLookupKey('twitch', 'xqc')).not.toContain(':');
  });

  it('keeps the platforms apart despite folding case', () => {
    expect(sliceLookupKey('kick', 'xqc')).not.toBe(sliceLookupKey('twitch', 'xqc'));
  });

  // The reverse of the change that broke chat: if anything ever makes this
  // case-preserving again, the lookup stops matching storage and every YouTube
  // row is dropped.
  it('is NOT the case-preserving composite', () => {
    expect(sliceLookupKey('youtube', 'HVtwmO9RLNw')).not.toBe(makeKey('youtube', 'HVtwmO9RLNw'));
  });
});
