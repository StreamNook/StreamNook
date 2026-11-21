import { describe, expect, it } from 'vitest';
import { HYPE_LOOK_FILL, hypeTrainLook } from './hypeTrainLook';

describe('hypeTrainLook', () => {
  it('reads the look from the train', () => {
    expect(hypeTrainLook({ is_golden_kappa: true })).toBe('golden');
    expect(hypeTrainLook({ is_golden_kappa: false })).toBe('standard');
    expect(hypeTrainLook({})).toBe('standard');
  });

  it('gives every look a fill', () => {
    expect(HYPE_LOOK_FILL.standard).toBe('hype-train-progress-rainbow');
    expect(HYPE_LOOK_FILL.golden).toBe('hype-train-progress-golden');
  });
});
