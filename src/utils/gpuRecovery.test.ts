import { afterEach, describe, expect, it, vi } from 'vitest';

const { addToast, listen } = vi.hoisted(() => ({
  addToast: vi.fn(),
  listen: vi.fn(async (_event: string, _handler: (e: { payload: number }) => void) => () => {}),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }));
vi.mock('@tauri-apps/api/event', () => ({ listen }));
vi.mock('../stores/AppStore', () => ({ useAppStore: { getState: () => ({ addToast }) } }));
vi.mock('../services/sessionResume', () => ({ captureResumeSnapshot: vi.fn(async () => {}) }));

const { GPU_RESTART_TOAST_ID, readDrawingMode, shouldOfferRestart, startGpuRecoveryWatch } = await import('./gpuRecovery');

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

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  addToast.mockClear();
  listen.mockClear();
});

describe('the restart toast', () => {
  it('goes out under one fixed id for every driver reset', async () => {
    vi.useFakeTimers();
    // Hardware at start, software at every check after.
    let strict: unknown = fakeGl('ANGLE (NVIDIA GeForce RTX 3070)');
    let loose: unknown = fakeGl('x');
    vi.stubGlobal('document', {
      createElement: () => ({
        getContext: (_kind: string, opts?: { failIfMajorPerformanceCaveat?: boolean }) =>
          opts?.failIfMajorPerformanceCaveat ? strict : loose,
      }),
    });
    const stop = startGpuRecoveryWatch();
    await vi.advanceTimersByTimeAsync(0);
    expect(listen).toHaveBeenCalledTimes(1);
    const onFailed = listen.mock.calls[0][1];
    strict = null;
    loose = fakeGl('Google SwiftShader');

    for (let i = 0; i < 3; i++) onFailed({ payload: 1 });
    await vi.advanceTimersByTimeAsync(10_000);

    expect(addToast).toHaveBeenCalledTimes(3);
    for (const call of addToast.mock.calls) {
      expect(call[3]).toMatchObject({ sticky: true, alwaysShow: true, id: GPU_RESTART_TOAST_ID });
    }
    expect(GPU_RESTART_TOAST_ID).toBeLessThan(0);
    stop();
  });
});

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
