import { describe, expect, it } from 'vitest';
import { createRevealRise } from './revealRise';

describe('createRevealRise', () => {
  it('rises when shown from an uncovered shell', () => {
    const rise = createRevealRise(false, false);
    expect(rise(true, false)).toBe(true);
  });

  it('does not rise when the cover leaves in the same render that shows it (Close video)', () => {
    const rise = createRevealRise(false, true);
    expect(rise(true, false)).toBe(false);
  });

  it('does not rise when shown under the cover, nor when the cover leaves later', () => {
    const rise = createRevealRise(false, true);
    expect(rise(true, true)).toBe(false);
    expect(rise(true, false)).toBe(false);
  });

  it('rises once per showing, and again after it was hidden', () => {
    const rise = createRevealRise(false, false);
    expect(rise(true, false)).toBe(true);
    expect(rise(true, false)).toBe(false);
    expect(rise(false, false)).toBe(false);
    expect(rise(true, false)).toBe(true);
  });
});
