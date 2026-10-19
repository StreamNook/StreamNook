import { describe, expect, it } from 'vitest';
import { isCodecFailure } from './codecFailure';

const video = (code?: number) => ({ error: code === undefined ? null : ({ code } as MediaError) });

describe('isCodecFailure', () => {
  it('counts the errors that mean the codec cannot play here at all', () => {
    expect(isCodecFailure('bufferIncompatibleCodecsError', video(), true)).toBe(true);
    expect(isCodecFailure('bufferAddCodecError', video(), false)).toBe(true);
    expect(isCodecFailure('manifestIncompatibleCodecsError', video(), false)).toBe(true);
  });

  it('counts a decode failure only before the first frame', () => {
    expect(isCodecFailure('bufferAppendError', video(3), false)).toBe(true);
    expect(isCodecFailure('bufferAppendError', video(4), false)).toBe(true);
    expect(isCodecFailure('bufferAppendError', video(3), true)).toBe(false);
  });

  it('leaves every other media error to the normal recovery', () => {
    expect(isCodecFailure('bufferAppendError', video(), false)).toBe(false);
    expect(isCodecFailure('fragParsingError', video(2), false)).toBe(false);
  });
});
