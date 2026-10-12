import { describe, expect, it } from 'vitest';
import { isImeComposing } from './imeComposing';

describe('isImeComposing', () => {
  it('reads a React event through its native event', () => {
    expect(isImeComposing({ nativeEvent: { isComposing: true, keyCode: 13 } })).toBe(true);
    expect(isImeComposing({ nativeEvent: { isComposing: false, keyCode: 13 } })).toBe(false);
  });

  it('treats keyCode 229 as composing when the flag is missing', () => {
    expect(isImeComposing({ nativeEvent: { keyCode: 229 } })).toBe(true);
    expect(isImeComposing({ keyCode: 229 })).toBe(true);
  });

  it('reads a DOM event directly', () => {
    expect(isImeComposing({ isComposing: true })).toBe(true);
    expect(isImeComposing({ isComposing: false, keyCode: 13 })).toBe(false);
  });
});
