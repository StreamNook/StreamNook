import { Logger } from './logger';

const REPORT_MS = 60_000;
/** Samples kept per stage per minute; later values replace earlier ones at random (reservoir). */
const SAMPLE_CAP = 512;
/** Rows waiting for their paint; the oldest are forgotten past this. */
const PENDING_CAP = 4096;

interface Stage {
  seen: number;
  max: number;
  sample: number[];
}

interface Pending {
  channel: string;
  receivedMs: number;
  twitchToRust: number | null;
  rustToPage: number | null;
}

/** The fields of a chat row frame this module reads. */
export interface LatencyRow {
  id?: unknown;
  channel?: unknown;
  received_at_ms?: unknown;
  tags?: unknown;
}

const newStage = (): Stage => ({ seen: 0, max: 0, sample: [] });

let twitchToRust = newStage();
let rustToPage = newStage();
let pageToPaint = newStage();
let rowsInMinute = 0;
let worst: { channel: string; ms: number } | null = null;
let minTwitchToRust: number | null = null;
let heldInMinute = 0;
let oldestHeldMs: number | null = null;
const pending = new Map<string, Pending>();

function addSample(stage: Stage, value: number): void {
  stage.seen += 1;
  if (stage.seen === 1 || value > stage.max) stage.max = value;
  if (stage.sample.length < SAMPLE_CAP) {
    stage.sample.push(value);
    return;
  }
  const slot = Math.floor(Math.random() * stage.seen);
  if (slot < SAMPLE_CAP) stage.sample[slot] = value;
}

function tagValue(tags: unknown, name: string): string | undefined {
  if (tags instanceof Map) {
    const v = tags.get(name);
    return typeof v === 'string' ? v : undefined;
  }
  if (tags && typeof tags === 'object') {
    const v = (tags as Record<string, unknown>)[name];
    return typeof v === 'string' ? v : undefined;
  }
  return undefined;
}

/** Nearest-rank percentile of an ascending list. */
function rank(sorted: number[], p: number): number {
  return sorted[Math.min(sorted.length - 1, Math.max(0, Math.ceil(p * sorted.length) - 1))];
}

/** p50, p95 and max of a stage (ms, rounded). */
export function stageSummary(samples: number[], max: number): { p50: number; p95: number; max: number } {
  if (samples.length === 0) return { p50: 0, p95: 0, max: 0 };
  const sorted = [...samples].sort((a, b) => a - b);
  return { p50: Math.round(rank(sorted, 0.5)), p95: Math.round(rank(sorted, 0.95)), max: Math.round(max) };
}

function describeStage(stage: Stage): string {
  if (stage.seen === 0) return 'n/a';
  const s = stageSummary(stage.sample, stage.max);
  return `${s.p50}/${s.p95}/${s.max}ms`;
}

/**
 * A chat row frame reached this page's bridge socket at `nowMs` (local clock).
 * A `held` row waited in a parked tab and is released on unpark: it is counted
 * with its age and kept out of every stage, so it never reads as live latency.
 */
export function noteRowReceived(row: LatencyRow, nowMs: number, held = false): void {
  if (typeof row.id !== 'string' || row.id === '') return;

  const sentMs = Number(tagValue(row.tags, 'tmi-sent-ts'));
  const readMs = typeof row.received_at_ms === 'number' && Number.isFinite(row.received_at_ms) ? row.received_at_ms : null;
  if (held) {
    heldInMinute += 1;
    const originMs = readMs ?? (Number.isFinite(sentMs) && sentMs > 0 ? sentMs : null);
    if (originMs !== null) {
      const age = nowMs - originMs;
      if (oldestHeldMs === null || age > oldestHeldMs) oldestHeldMs = age;
    }
    return;
  }
  rowsInMinute += 1;

  let t2r: number | null = null;
  let r2p: number | null = null;
  if (readMs !== null) {
    r2p = nowMs - readMs;
    addSample(rustToPage, r2p);
    if (Number.isFinite(sentMs) && sentMs > 0) {
      t2r = readMs - sentMs;
      addSample(twitchToRust, t2r);
      if (minTwitchToRust === null || t2r < minTwitchToRust) minTwitchToRust = t2r;
    }
  }

  pending.set(row.id, {
    channel: typeof row.channel === 'string' ? row.channel : '',
    receivedMs: nowMs,
    twitchToRust: t2r,
    rustToPage: r2p,
  });
  if (pending.size > PENDING_CAP) {
    const oldest = pending.keys().next().value;
    if (oldest !== undefined) pending.delete(oldest);
  }
}

/** Rows with these ids were drawn at `paintMs` (local clock). Unknown ids are ignored. */
export function noteRowsPainted(ids: readonly string[], paintMs: number): void {
  for (const id of ids) {
    const row = pending.get(id);
    if (!row) continue;
    pending.delete(id);
    const paint = Math.max(0, paintMs - row.receivedMs);
    addSample(pageToPaint, paint);
    // The slowest row by the sum of the stages it has numbers for; a negative
    // server-clock stage counts as zero.
    const total = Math.max(0, row.twitchToRust ?? 0) + Math.max(0, row.rustToPage ?? 0) + paint;
    if (worst === null || total > worst.ms) worst = { channel: row.channel, ms: Math.round(total) };
  }
}

/** The line for the minute so far, or null when no row arrived; starts a new minute either way. */
export function takeChatLatencyLine(): string | null {
  const n = rowsInMinute;
  const heldField =
    heldInMinute === 0
      ? ''
      : `, held ${heldInMinute} (oldest ${oldestHeldMs === null ? 'n/a' : `${Math.round(oldestHeldMs)}ms`})`;
  const line =
    n === 0 && heldInMinute === 0
      ? null
      : `[ChatLatency] ${n} rows, twitch->rust ${describeStage(twitchToRust)}, rust->page ${describeStage(rustToPage)}, page->paint ${describeStage(pageToPaint)}, worst ${
          worst ? `${worst.channel || 'unknown'} ${worst.ms}ms` : 'n/a'
        }, clock skew est ${minTwitchToRust === null ? 'n/a' : `${Math.round(minTwitchToRust)}ms`}${heldField}`;
  heldInMinute = 0;
  oldestHeldMs = null;
  twitchToRust = newStage();
  rustToPage = newStage();
  pageToPaint = newStage();
  rowsInMinute = 0;
  worst = null;
  minTwitchToRust = null;
  return line;
}

/** How much memory the tracker holds, for tests. */
export function latencyHeld(): { twitchToRust: number; rustToPage: number; pageToPaint: number; pending: number } {
  return {
    twitchToRust: twitchToRust.sample.length,
    rustToPage: rustToPage.sample.length,
    pageToPaint: pageToPaint.sample.length,
    pending: pending.size,
  };
}

/** Forget everything, for tests. */
export function resetChatLatency(): void {
  takeChatLatencyLine();
  pending.clear();
}

let reporterTimer: ReturnType<typeof setInterval> | null = null;

/** Once a minute while rows arrive: how long rows took from Twitch to Rust, Rust to this
 *  page, and this page to paint. Warn level, so it reaches every report. */
export function startChatLatencyReporter(): void {
  if (reporterTimer !== null) return;
  reporterTimer = setInterval(() => {
    const line = takeChatLatencyLine();
    if (line) Logger.warn(line);
  }, REPORT_MS);
}

/** Stop the reporter, for tests. */
export function stopChatLatencyReporter(): void {
  if (reporterTimer === null) return;
  clearInterval(reporterTimer);
  reporterTimer = null;
}
