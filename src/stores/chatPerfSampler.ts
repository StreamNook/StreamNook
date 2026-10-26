// Live chat timing for the log: how old a Twitch row is when this window
// queues it (Twitch's send time to here), the longest wait for the next
// paint, and the long frames that held the page up, by the script behind
// them. Quiet while chat is healthy: a line when something was slow, and a
// baseline every ten minutes.

const WINDOW_MS = 60_000;
const BASELINE_EVERY_MS = 10 * 60_000;
const RESERVOIR = 1000;
const SLOW_SPREAD_MS = 1000;
const SLOW_PAINT_MS = 500;
const LONG_FRAME_MS = 200;
/** Frames shorter than this are not reported at all. */
const REPORTED_FRAME_MS = 100;

export type ChatPerfSampler = ReturnType<typeof createChatPerfSampler>;

export function createChatPerfSampler(rng: () => number = Math.random) {
  let ages: number[] = [];
  let seen = 0;
  let maxAge = -Infinity;
  let windowStart = 0;
  let lastLineAt = 0;
  let paintWaitMax = 0;
  let frameCount = 0;
  let frameWorst = 0;
  let byScript = new Map<string, number>();

  return {
    /** History, replays and a badly set clock fall outside the window. */
    noteRowAge(ageMs: number, now = Date.now()): void {
      if (!Number.isFinite(ageMs) || ageMs < -10_000 || ageMs > 60_000) return;
      if (windowStart === 0) windowStart = now;
      seen++;
      maxAge = Math.max(maxAge, ageMs);
      // A busy room sends more rows a minute than are kept: every row has the
      // same chance of being in the sample, not just the minute's first ones.
      if (ages.length < RESERVOIR) {
        ages.push(ageMs);
      } else {
        const j = Math.floor(rng() * seen);
        if (j < RESERVOIR) ages[j] = ageMs;
      }
    },

    notePaintWait(ms: number): void {
      if (ms > paintWaitMax) paintWaitMax = ms;
    },

    noteLongFrame(ms: number, script: string | null): void {
      frameCount++;
      frameWorst = Math.max(frameWorst, ms);
      if (script) byScript.set(script, Math.max(byScript.get(script) ?? 0, ms));
    },

    /** The minute's line when it is due and worth writing, else null. A minute
     *  that has passed starts the next one either way. */
    take(label: string, now = Date.now()): string | null {
      if (windowStart === 0 || now - windowStart < WINDOW_MS || ages.length === 0) return null;
      const sorted = [...ages].sort((a, b) => a - b);
      const at = (q: number) => Math.round(sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))]);
      // The fastest row stands in for the clock difference between this
      // machine and Twitch, so the spread over it is the delay itself.
      const spread = at(0.95) - Math.round(sorted[0]);
      const slow = spread >= SLOW_SPREAD_MS || paintWaitMax >= SLOW_PAINT_MS || frameWorst >= LONG_FRAME_MS;
      const top = [...byScript]
        .sort((a, b) => b[1] - a[1])
        .slice(0, 3)
        .map(([k, v]) => `${k} ${Math.round(v)}ms`)
        .join(', ');
      const line =
        slow || lastLineAt === 0 || now - lastLineAt >= BASELINE_EVERY_MS
          ? `[ChatPerf] ${label}${slow ? ' slow' : ''}: ${seen} live rows, age p50 ${at(0.5)}ms p95 ${at(0.95)}ms max ${Math.round(maxAge)}ms ` +
            `(p95 over fastest ${spread}ms), paint wait max ${Math.round(paintWaitMax)}ms, long frames ${frameCount}` +
            (frameCount ? ` worst ${Math.round(frameWorst)}ms${top ? ` (${top})` : ''}` : '')
          : null;
      if (line) lastLineAt = now;
      ages = [];
      seen = 0;
      maxAge = -Infinity;
      windowStart = 0;
      paintWaitMax = 0;
      frameCount = 0;
      frameWorst = 0;
      byScript = new Map();
      return line;
    },
  };
}

type ScriptTiming = { invoker: string; duration: number; sourceCharPosition: number };

/** Frames of 100 ms or more, from Long Animation Frames where the engine has
 *  it (Chromium 123+: WebView2, CEF, Android WebView), else Long Tasks, else
 *  nothing (WebKit). Each is named by its slowest script and where in the
 *  bundle that script starts. */
export function watchLongFrames(onFrame: (ms: number, script: string | null) => void): void {
  const types: readonly string[] =
    typeof PerformanceObserver !== 'undefined' ? (PerformanceObserver.supportedEntryTypes ?? []) : [];
  const type = types.includes('long-animation-frame')
    ? 'long-animation-frame'
    : types.includes('longtask')
      ? 'longtask'
      : null;
  if (!type) return;
  try {
    new PerformanceObserver((list) => {
      for (const e of list.getEntries()) {
        if (e.duration < REPORTED_FRAME_MS) continue;
        const scripts = (e as PerformanceEntry & { scripts?: ScriptTiming[] }).scripts ?? [];
        const worst = scripts.reduce<ScriptTiming | null>((w, s) => (!w || s.duration > w.duration ? s : w), null);
        onFrame(e.duration, worst ? `${worst.invoker.slice(0, 60)}@${worst.sourceCharPosition}` : null);
      }
    }).observe({ type });
  } catch {
    // The engine listed the type but refused the observer: no frame data.
  }
}
