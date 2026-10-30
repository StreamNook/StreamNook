import { describe, expect, it } from 'vitest';
import { elapsedText } from './discordPreviewFormat';

describe('elapsedText', () => {
  it('counts like Discord', () => {
    const start = 1_700_000_000;
    expect(elapsedText(start, (start + 5) * 1000)).toBe('00:05');
    expect(elapsedText(start, (start + 12 * 60 + 4) * 1000)).toBe('12:04');
    expect(elapsedText(start, (start + 72 * 60 + 44) * 1000)).toBe('01:12:44');
    expect(elapsedText(start, (start - 30) * 1000)).toBe('00:00');
  });
});
