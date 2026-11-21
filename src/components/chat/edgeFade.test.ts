import { describe, expect, it } from 'vitest';
import { FADE_PX, edgeFade, readEdges } from './edgeFade';

describe('readEdges', () => {
  it('has no edge where the content fits or the scroller sits at both ends', () => {
    expect(readEdges({ scrollTop: 0, scrollHeight: 100, clientHeight: 100 })).toEqual({ top: false, bottom: false });
  });

  it('has an edge only where more than 2 px lies beyond it', () => {
    expect(readEdges({ scrollTop: 0, scrollHeight: 300, clientHeight: 100 })).toEqual({ top: false, bottom: true });
    expect(readEdges({ scrollTop: 100, scrollHeight: 300, clientHeight: 100 })).toEqual({ top: true, bottom: true });
    expect(readEdges({ scrollTop: 200, scrollHeight: 300, clientHeight: 100 })).toEqual({ top: true, bottom: false });
    expect(readEdges({ scrollTop: 1, scrollHeight: 101, clientHeight: 100 })).toEqual({ top: false, bottom: false });
  });
});

describe('edgeFade', () => {
  it('draws no mask for a scroller that fits, so a card keeps its own edge and corners', () => {
    expect(edgeFade({ top: false, bottom: false })).toBeUndefined();
  });

  it('fades only the edges that have more beyond them', () => {
    const bottom = edgeFade({ top: false, bottom: true })?.maskImage as string;
    expect(bottom).toContain(`#000 0px`);
    expect(bottom).toContain(`calc(100% - ${FADE_PX}px)`);
    const top = edgeFade({ top: true, bottom: false })?.maskImage as string;
    expect(top).toContain(`#000 ${FADE_PX}px`);
    expect(top).toContain('calc(100% - 0px)');
    const both = edgeFade({ top: true, bottom: true })?.maskImage as string;
    expect(both).toContain(`#000 ${FADE_PX}px`);
    expect(both).toContain(`calc(100% - ${FADE_PX}px)`);
  });

  it('carries the prefixed property for the older WebView', () => {
    const fade = edgeFade({ top: true, bottom: true });
    expect(fade?.WebkitMaskImage).toBe(fade?.maskImage);
  });
});
