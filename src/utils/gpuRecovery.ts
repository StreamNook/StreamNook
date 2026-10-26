import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useAppStore } from '../stores/AppStore';
import { captureResumeSnapshot } from '../services/sessionResume';
import { Logger } from './logger';

export type DrawingMode = 'hardware' | 'software' | 'unknown';

const SOFTWARE_RENDERER = /SwiftShader|Basic Render|llvmpipe|Software/i;

function rendererOf(gl: WebGLRenderingContext): string {
  const ext = gl.getExtension('WEBGL_debug_renderer_info');
  const name = ext ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER);
  gl.getExtension('WEBGL_lose_context')?.loseContext();
  return String(name ?? '');
}

/** How the page draws, from WebGL. A context refused under
 *  `failIfMajorPerformanceCaveat` while a plain one works is software
 *  (SwiftShader); no context at all is software too (GPU process without GL). */
export function readDrawingMode(doc: Document = document): { mode: DrawingMode; renderer: string } {
  try {
    const strict = doc.createElement('canvas').getContext('webgl', { failIfMajorPerformanceCaveat: true }) as WebGLRenderingContext | null;
    if (strict) {
      const renderer = rendererOf(strict);
      return { mode: SOFTWARE_RENDERER.test(renderer) ? 'software' : 'hardware', renderer };
    }
    const loose = doc.createElement('canvas').getContext('webgl') as WebGLRenderingContext | null;
    if (!loose) return { mode: 'software', renderer: 'none' };
    return { mode: 'software', renderer: rendererOf(loose) };
  } catch {
    return { mode: 'unknown', renderer: '' };
  }
}

/** Offer the restart only when drawing fell from the GPU to software. */
export function shouldOfferRestart(atStart: DrawingMode, checks: DrawingMode[]): boolean {
  return atStart === 'hardware' && checks.length > 0 && checks.every((m) => m === 'software');
}

const RECHECK_MS = [3000, 10_000];

/** The restart toast's id. Toast ids otherwise come from `Date.now() +
 *  Math.random()`, which is never negative, so this cannot collide. Every
 *  driver reset shows the one toast instead of stacking another. */
export const GPU_RESTART_TOAST_ID = -1;

async function restartForDrawing(): Promise<void> {
  await captureResumeSnapshot();
  if (import.meta.env.DEV) {
    window.location.reload();
    return;
  }
  await invoke('restart_app', { reason: 'drawing fell back to software' });
}

/** Log the drawing mode now, and after every GPU process failure; offer the
 *  restart when it came back without the GPU. Returns the cleanup. */
export function startGpuRecoveryWatch(): () => void {
  const start = readDrawingMode();
  // Warn level: this line answers "how was it drawing" in every report.
  Logger.warn(`[GPU] drawing: ${start.mode} (${start.renderer || 'unknown renderer'})`);
  let disposed = false;
  let unlisten: (() => void) | null = null;
  const timers: ReturnType<typeof setTimeout>[] = [];
  void listen<number>('webview-gpu-failed', (event) => {
    const checks: DrawingMode[] = [];
    for (const [i, delay] of RECHECK_MS.entries()) {
      timers.push(
        setTimeout(() => {
          if (disposed) return;
          const now = readDrawingMode();
          checks.push(now.mode);
          Logger.warn(
            `[GPU] after the GPU process failed (exit ${event.payload}), check ${i + 1}: drawing ${now.mode} (${now.renderer || 'unknown renderer'}); at start ${start.mode}`,
          );
          if (i === RECHECK_MS.length - 1 && shouldOfferRestart(start.mode, checks)) {
            useAppStore.getState().addToast(
              'Video and chat are drawing without the graphics card since a graphics driver reset, which makes them lag. Restart StreamNook to fix it; your stream reopens.',
              'warning',
              { label: 'Restart', onClick: () => void restartForDrawing() },
              { sticky: true, alwaysShow: true, id: GPU_RESTART_TOAST_ID },
            );
          }
        }, delay),
      );
    }
  }).then((u) => {
    if (disposed) u();
    else unlisten = u;
  });
  return () => {
    disposed = true;
    unlisten?.();
    for (const t of timers) clearTimeout(t);
  };
}
