// The hold mark is read once (SN-78: a tag pill and the card both consume it).
import { describe, expect, it } from 'vitest';
import { createHoldFlag } from './holdFlag';

describe('hold flag', () => {
  it('is read once after a hold', () => {
    const flag = createHoldFlag();
    expect(flag.consume()).toBe(false);
    flag.set();
    expect(flag.consume()).toBe(true);
    expect(flag.consume()).toBe(false);
  });

  it('a new press clears it', () => {
    const flag = createHoldFlag();
    flag.set();
    flag.clear();
    expect(flag.consume()).toBe(false);
  });
});
