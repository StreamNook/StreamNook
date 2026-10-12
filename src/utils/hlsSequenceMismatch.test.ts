import { describe, expect, it, vi } from 'vitest';
import Hls from 'hls.js';
import type { ErrorData } from 'hls.js';
import { recoverFromSequenceMismatch } from './hlsSequenceMismatch';

function fakeHls() {
  const listeners: Array<() => void> = [];
  const hls = {
    config: { ignorePlaylistParsingErrors: false } as Hls['config'],
    once: vi.fn((_event: string, fn: () => void) => {
      listeners.push(fn);
    }),
    startLoad: vi.fn(),
  };
  return { hls: hls as unknown as Pick<Hls, 'config' | 'once' | 'startLoad'>, raw: hls, fire: () => listeners.forEach(fn => fn()) };
}

const mismatch = {
  type: Hls.ErrorTypes.NETWORK_ERROR,
  details: Hls.ErrorDetails.LEVEL_PARSING_ERROR,
  fatal: true,
  error: new Error('media sequence mismatch 322: http://localhost:18687/pseg/78'),
} as unknown as ErrorData;

describe('recoverFromSequenceMismatch', () => {
  it('accepts exactly the next refresh and resumes loading', () => {
    const { hls, raw, fire } = fakeHls();
    expect(recoverFromSequenceMismatch(hls, mismatch)).toBe(true);
    expect(raw.config.ignorePlaylistParsingErrors).toBe(true);
    expect(raw.startLoad).toHaveBeenCalledTimes(1);
    expect(raw.once).toHaveBeenCalledWith(Hls.Events.LEVEL_UPDATED, expect.any(Function));
    fire();
    expect(raw.config.ignorePlaylistParsingErrors).toBe(false);
  });

  it('leaves other playlist failures to the caller', () => {
    const { hls, raw } = fakeHls();
    const unparsable = { ...mismatch, error: new Error('Missing Target Duration') } as ErrorData;
    const loadError = { ...mismatch, details: Hls.ErrorDetails.LEVEL_LOAD_ERROR } as ErrorData;
    expect(recoverFromSequenceMismatch(hls, unparsable)).toBe(false);
    expect(recoverFromSequenceMismatch(hls, loadError)).toBe(false);
    expect(raw.startLoad).not.toHaveBeenCalled();
    expect(raw.config.ignorePlaylistParsingErrors).toBe(false);
  });
});
