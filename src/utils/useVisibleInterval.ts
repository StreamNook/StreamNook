import { useEffect, useRef } from 'react';
import { isWindowHidden, onWindowVisibility } from './windowVisibility';

/**
 * setInterval-style polling that skips ticks while the StreamNook window is
 * hidden (minimized, in the tray, or backgrounded). When the window becomes
 * visible again the callback fires immediately so the UI catches up.
 *
 * Use this for any polling that's purely about updating an on-screen surface.
 * Don't use it for things that need to run regardless of visibility (e.g.
 * background heartbeats Twitch needs to see — those should stay in Rust).
 *
 * `fn` is captured by ref (synced in an effect, never on the render path) so
 * its identity doesn't need to be stable across renders, which lets callers
 * pass an inline async function without forcing a wrapping useCallback.
 */
export function useVisibleInterval(fn: () => void | Promise<void>, ms: number) {
  const fnRef = useRef(fn);
  useEffect(() => {
    fnRef.current = fn;
  });

  useEffect(() => {
    let cancelled = false;

    const run = () => {
      if (cancelled) return;
      // Rust-aware: a minimized window still reads as visible to Chromium here.
      if (isWindowHidden()) return;
      // We don't await — preserves setInterval's fire-and-forget semantics and
      // matches the existing setInterval call sites this hook replaces.
      void fnRef.current();
    };

    const id = setInterval(run, ms);

    // When the window becomes visible after being hidden, fire immediately
    // so the UI doesn't have to wait up to `ms` for the next tick.
    const onVisibilityChange = () => {
      if (!isWindowHidden()) run();
    };
    const off = onWindowVisibility(onVisibilityChange);
    return () => {
      cancelled = true;
      clearInterval(id);
      off();
    };
  }, [ms]);
}
