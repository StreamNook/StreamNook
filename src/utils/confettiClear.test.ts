import { describe, expect, it } from 'vitest';
import { confettiClearMs } from './confettiClear';

describe('confettiClearMs', () => {
  it('keeps confetti until brushed on a fresh install', () => {
    expect(confettiClearMs(undefined)).toBeNull();
    expect(confettiClearMs({} as never)).toBeNull();
    expect(confettiClearMs({ confetti_auto_clear: false, confetti_clear_seconds: 8 } as never)).toBeNull();
  });

  it('clears after 5 seconds once turned on', () => {
    expect(confettiClearMs({ confetti_auto_clear: true } as never)).toBe(5000);
  });

  it('uses the chosen delay, held to 3 to 15 seconds', () => {
    expect(confettiClearMs({ confetti_auto_clear: true, confetti_clear_seconds: 9 } as never)).toBe(9000);
    expect(confettiClearMs({ confetti_auto_clear: true, confetti_clear_seconds: 1 } as never)).toBe(3000);
    expect(confettiClearMs({ confetti_auto_clear: true, confetti_clear_seconds: 60 } as never)).toBe(15000);
  });
});
