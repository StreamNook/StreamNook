import { describe, expect, it } from 'vitest';
import { pickFittingText } from './placeholderFit';

// 8 px per character stands in for a real font.
const widthOf = (text: string) => text.length * 8;

describe('pickFittingText', () => {
  it('keeps the full wording when it fits', () => {
    expect(pickFittingText(['Slow mode (5 seconds)', 'Slow 5s'], 400, widthOf)).toBe('Slow mode (5 seconds)');
  });

  it('falls back to the short wording when the full one would wrap', () => {
    // The landscape side chat on a 2388x1080 phone: the full prompt wrapped.
    expect(pickFittingText(['Slow mode (5 seconds)', 'Slow 5s'], 120, widthOf)).toBe('Slow 5s');
  });

  it('says the shortest wording even when nothing fits', () => {
    expect(pickFittingText(['Slow mode (5 seconds)', 'Slow 5s'], 20, widthOf)).toBe('Slow 5s');
  });

  it('uses the full wording before the field is measured', () => {
    expect(pickFittingText(['Slow mode (5 seconds)', 'Slow 5s'], 0, widthOf)).toBe('Slow mode (5 seconds)');
  });

  it('skips empty wordings and takes the first when nothing can be measured', () => {
    expect(pickFittingText(['', 'Send a message'], 40, widthOf)).toBe('Send a message');
    expect(pickFittingText(['Slow mode (5 seconds)', 'Slow 5s'], 40, () => null)).toBe('Slow mode (5 seconds)');
    expect(pickFittingText([], 40, widthOf)).toBe('');
  });
});
