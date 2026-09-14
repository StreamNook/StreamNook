// Development-only: switch off React 19's per-element dev instrumentation
// before react-dom evaluates. This module MUST stay the first import in
// main.tsx (ES modules evaluate imports depth-first in source order, so
// anything imported above it would pull react-dom in first).
//
// React 19.2 development builds do two things per element / render that
// React 18 did not, and both are gated on globals we control:
//   1. Performance Tracks: a performance.measure() per render, commit and
//      effect, gated on `console.timeStamp` being a function.
//   2. Owner tasks: a `console.createTask()` per created element, gated on
//      `console.createTask` being a function (owner STACKS, the per-element
//      Error object, are not gated and stay).
// Measured 2026-09-05 (1,500 tooltip-wrapped tiles, 20 re-renders, dev
// build, StrictMode, headless Chromium): React 18 about 67 ms per re-render,
// React 19 about 90 ms, React 19 with these two off about 75 ms. Production
// builds do none of this, so this file is stripped by the DEV guard.
//
// Keep the instrumentation for a profiling session by setting
// localStorage 'sn-react-devtracks' = '1' before reload; the scratchpad
// `cdp.mjs measures` recipe depends on the tracks being on.
if (import.meta.env.DEV) {
  let keep = false;
  try {
    keep = localStorage.getItem('sn-react-devtracks') === '1';
  } catch {
    keep = false;
  }
  if (!keep) {
    const c = console as unknown as Record<string, unknown>;
    c.timeStamp = undefined;
    c.createTask = undefined;
  }
}

export {};
