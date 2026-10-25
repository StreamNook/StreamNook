import { Logger } from './logger';
import { isWindowHidden } from './windowVisibility';

const TICK_MS = 250;
const REPORT_MS = 60_000;

/** p95 and max of lateness samples (ms), rounded. */
export function latenessSummary(samples: number[]): { p95: number; max: number } {
  if (samples.length === 0) return { p95: 0, max: 0 };
  const s = [...samples].sort((a, b) => a - b);
  const p95 = s[Math.min(s.length - 1, Math.floor(s.length * 0.95))];
  return { p95: Math.round(p95), max: Math.round(s[s.length - 1]) };
}

let started = false;

/** Once a minute: how late the page's main thread ran a 250 ms timer (what
 *  chat waits on) and its long tasks. Skips a hidden window, whose timers the
 *  browser throttles. Warn level, so it reaches every report. */
export function startPageHeartbeat(): void {
  if (started || typeof window === 'undefined') return;
  started = true;
  let late: number[] = [];
  let longTasks = 0;
  let longest = 0;
  try {
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        longTasks += 1;
        longest = Math.max(longest, entry.duration);
      }
    }).observe({ type: 'longtask', buffered: false });
  } catch {
    /* no long-task timing in this engine */
  }
  let expected = performance.now() + TICK_MS;
  const tick = () => {
    const now = performance.now();
    if (!isWindowHidden()) late.push(Math.max(0, now - expected));
    expected = now + TICK_MS;
    setTimeout(tick, TICK_MS);
  };
  setTimeout(tick, TICK_MS);
  setInterval(() => {
    if (late.length === 0) return;
    const { p95, max } = latenessSummary(late);
    Logger.warn(
      `[ChatPerf] page main thread: late p95 ${p95}ms max ${max}ms (n=${late.length}); long tasks ${longTasks} (max ${Math.round(longest)}ms)`,
    );
    late = [];
    longTasks = 0;
    longest = 0;
  }, REPORT_MS);
}
