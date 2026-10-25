import { describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('../stores/AppStore', () => ({ useAppStore: { getState: () => ({ addToast: vi.fn() }) } }));
vi.mock('../services/sessionResume', () => ({ captureResumeSnapshot: vi.fn(async () => {}) }));

const { readDrawingMode, shouldOfferRestart } = await import('./gpuRecovery');

/** A document whose canvases hand out `strict` for the
 *  failIfMajorPerformanceCaveat request and `loose` otherwise. */
function fakeDoc(strict: unknown, loose: unknown): Document {
  return {
    createElement: () => ({
      getContext: (_kind: string, opts?: { failIfMajorPerformanceCaveat?: boolean }) =>
        opts?.failIfMajorPerformanceCaveat ? strict : loose,
    }),
  } as unknown as Document;
}

function fakeGl(renderer: string) {
  return {
    RENDERER: 0x1f01,
    getExtension: (name: string) =>
      name === 'WEBGL_debug_renderer_info'
        ? { UNMASKED_RENDERER_WEBGL: 0x9246 }
        : name === 'WEBGL_lose_context'
          ? { loseContext: () => {} }
          : null,
    getParameter: () => renderer,
  };
}

describe('gpuRecovery', () => {
  it('offers the restart only when drawing fell from the GPU to software', () => {
    expect(shouldOfferRestart('hardware', ['software', 'software'])).toBe(true);
    expect(shouldOfferRestart('hardware', ['software', 'hardware'])).toBe(false);
    expect(shouldOfferRestart('software', ['software', 'software'])).toBe(false);
    expect(shouldOfferRestart('hardware', [])).toBe(false);
  });

  it('reads the drawing mode from WebGL', () => {
    expect(readDrawingMode(fakeDoc(fakeGl('ANGLE (NVIDIA GeForce RTX 3070)'), fakeGl('x')))).toEqual({
      mode: 'hardware',
      renderer: 'ANGLE (NVIDIA GeForce RTX 3070)',
    });
    expect(readDrawingMode(fakeDoc(null, fakeGl('ANGLE (Google, Vulkan (SwiftShader Device))'))).mode).toBe('software');
    expect(readDrawingMode(fakeDoc(null, null))).toEqual({ mode: 'software', renderer: 'none' });
    expect(readDrawingMode(fakeDoc(fakeGl('Google SwiftShader'), null)).mode).toBe('software');
  });
});
