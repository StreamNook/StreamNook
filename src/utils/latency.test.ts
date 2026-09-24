import { describe, it, expect } from 'vitest';
import { AUTO_GAP, behindLiveFromEdge, edgeTargetForGap, LL_EDGE_DELAY, resolveLiveEdgeGap } from './latency';

describe('resolveLiveEdgeGap', () => {
  it('uses the automatic gap of the delivery path when nothing is set', () => {
    expect(resolveLiveEdgeGap(undefined, 'll')).toBe(AUTO_GAP.ll);
    expect(resolveLiveEdgeGap(null, 'promotion')).toBe(AUTO_GAP.promotion);
    expect(resolveLiveEdgeGap(undefined, 'plain')).toBe(AUTO_GAP.plain);
  });

  it('keeps a gap the viewer chose on every path', () => {
    for (const path of ['ll', 'promotion', 'plain'] as const) {
      expect(resolveLiveEdgeGap(3.2, path)).toBe(3.2);
    }
  });

  it('treats a nonsense value as unset', () => {
    expect(resolveLiveEdgeGap(0, 'll')).toBe(AUTO_GAP.ll);
    expect(resolveLiveEdgeGap(Number.NaN, 'plain')).toBe(AUTO_GAP.plain);
  });

  it('orders the automatic gaps by how thin each ride is', () => {
    expect(AUTO_GAP.ll).toBeLessThan(AUTO_GAP.promotion);
    expect(AUTO_GAP.promotion).toBeLessThan(AUTO_GAP.plain);
  });
});

describe('the edge model', () => {
  it('reads behind-live as the edge distance plus the measured edge delay', () => {
    expect(behindLiveFromEdge(1.7)).toBeCloseTo(1.7 + LL_EDGE_DELAY, 5);
    expect(behindLiveFromEdge(0)).toBe(LL_EDGE_DELAY);
  });

  it('turns a gap into the edge distance that lands it, never below zero', () => {
    expect(edgeTargetForGap(2.0)).toBeCloseTo(2.0 - LL_EDGE_DELAY, 5);
    expect(edgeTargetForGap(0.1)).toBe(0);
    expect(behindLiveFromEdge(edgeTargetForGap(AUTO_GAP.ll))).toBeCloseTo(AUTO_GAP.ll, 5);
  });
});
