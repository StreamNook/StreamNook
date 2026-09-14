import type { Window } from '@tauri-apps/api/window';
import { Logger } from './logger';

/**
 * Read the window's inner size in LOGICAL (CSS) pixels.
 *
 * Tauri's `innerSize()` returns PHYSICAL device pixels. Every offset in the
 * aspect-ratio formula (40px title bar, chat size, measured sidebar width,
 * MultiNook gaps) is a CSS pixel, so the window size has to be converted before it
 * can be mixed with them. Writing a physical number back as a `LogicalSize`
 * multiplies the window by the scale factor on every resize event, which is what
 * made the window grow off-screen on any display above 100% scaling.
 */
export const getLogicalInnerSize = async (
  win: Window,
): Promise<{ width: number; height: number; scale: number }> => {
  const scale = await win.scaleFactor();
  const size = await win.innerSize();
  return {
    width: Math.round(size.width / scale),
    height: Math.round(size.height / scale),
    scale,
  };
};

/**
 * Keep an automatic resize inside the current monitor's work area, preserving the
 * requested shape. Only the aspect-lock resizes go through this; a user dragging the
 * window edge is never clamped.
 *
 * `scale` is the WINDOW's scale factor when the caller measured `width`/`height`
 * (from getLogicalInnerSize). The clamped size is applied as a `LogicalSize`
 * interpreted in the window's scale, so the work area must be divided by that same
 * scale — using the monitor's own factor mis-clamps by the ratio whenever the
 * window and monitor disagree (mixed-DPI monitors, or a stale cached factor
 * mid-drag). Falls back to the monitor's factor when not provided.
 */
export const clampToWorkArea = async (
  width: number,
  height: number,
  scale?: number,
): Promise<{ width: number; height: number }> => {
  const MIN_WIDTH = 800;
  const MIN_HEIGHT = 600;
  try {
    const { currentMonitor, primaryMonitor } = await import('@tauri-apps/api/window');
    // currentMonitor() is legitimately null while the window straddles monitors;
    // clamping against the primary beats returning an unclamped size that can
    // span every screen.
    const monitor = (await currentMonitor()) ?? (await primaryMonitor());
    if (!monitor) {
      Logger.warn('[WindowSizing] No monitor available; skipping work-area clamp');
      return { width, height };
    }

    const s = scale ?? monitor.scaleFactor;
    const maxWidth = Math.floor(monitor.workArea.size.width / s);
    const maxHeight = Math.floor(monitor.workArea.size.height / s);

    const factor = Math.min(1, maxWidth / width, maxHeight / height);
    if (factor >= 1) return { width, height };

    return {
      width: Math.max(MIN_WIDTH, Math.round(width * factor)),
      height: Math.max(MIN_HEIGHT, Math.round(height * factor)),
    };
  } catch (error) {
    Logger.error('[WindowSizing] Failed to clamp to work area:', error);
    return { width, height };
  }
};
