import { describe, expect, it, vi } from 'vitest';

vi.mock('./windowVisibility', () => ({ isWindowHidden: () => false }));

const { latenessSummary } = await import('./pageHeartbeat');

describe('latenessSummary', () => {
  it('is zero for no samples', () => {
    expect(latenessSummary([])).toEqual({ p95: 0, max: 0 });
  });

  it('takes the 95th percentile and the maximum', () => {
    expect(latenessSummary(Array.from({ length: 100 }, (_, i) => i + 1))).toEqual({ p95: 96, max: 100 });
  });
});
