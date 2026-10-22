import { describe, expect, it } from 'vitest';
import { isCodecFailure } from './codecFailure';

describe('isCodecFailure', () => {
  it('counts the errors that mean the codec cannot play here at all', () => {
    expect(isCodecFailure('bufferIncompatibleCodecsError')).toBe(true);
    expect(isCodecFailure('bufferAddCodecError')).toBe(true);
    expect(isCodecFailure('manifestIncompatibleCodecsError')).toBe(true);
  });

  it('never counts a decode or append failure, which retries on the same codec', () => {
    expect(isCodecFailure('bufferAppendError')).toBe(false);
    expect(isCodecFailure('bufferAppendingError')).toBe(false);
    expect(isCodecFailure('fragParsingError')).toBe(false);
    expect(isCodecFailure('fragDecryptError')).toBe(false);
  });
});
