import { describe, expect, it } from 'vitest';
import { transientSwapVerdict } from './transientSwap';

describe('transientSwapVerdict', () => {
  const base = {
    seqAtStart: 3,
    seqNow: 3,
    streamUrlNow: 'http://127.0.0.1:1/x',
    loginAtStart: 'a',
    loginNow: 'a',
  };

  it('applies when nothing changed', () => {
    expect(transientSwapVerdict(base)).toBe('apply');
  });

  it('stops the relay when the stream closed mid-swap', () => {
    expect(transientSwapVerdict({ ...base, streamUrlNow: null })).toBe('discard-and-stop');
  });

  it('stops the relay when the stream object was cleared last', () => {
    expect(transientSwapVerdict({ ...base, loginNow: null })).toBe('discard-and-stop');
  });

  it('never stops a relay a newer start owns, even with the URL still null', () => {
    expect(transientSwapVerdict({ ...base, seqNow: 4, streamUrlNow: null })).toBe('discard');
  });

  it('never stops a relay a newer start owns when a different channel landed', () => {
    expect(transientSwapVerdict({ ...base, seqNow: 4, loginNow: 'b' })).toBe('discard');
  });
});
