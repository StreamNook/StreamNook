import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const warn = vi.fn();
vi.mock('./logger', () => ({ Logger: { warn: (...a: unknown[]) => warn(...a) } }));

const {
  latencyHeld,
  noteRowReceived,
  noteRowsPainted,
  resetChatLatency,
  stageSummary,
  startChatLatencyReporter,
  stopChatLatencyReporter,
  takeChatLatencyLine,
} = await import('./chatLatency');

/** A row Twitch stamped at `sent`, which Rust read at `read`. */
const row = (id: string, channel: string, sent: number, read?: number) => ({
  id,
  channel,
  tags: { 'tmi-sent-ts': String(sent) },
  ...(read === undefined ? {} : { received_at_ms: read }),
});

beforeEach(() => {
  vi.useFakeTimers();
  warn.mockClear();
  resetChatLatency();
});

afterEach(() => {
  stopChatLatencyReporter();
  vi.useRealTimers();
});

describe('stageSummary', () => {
  it('takes nearest-rank percentiles over a known set', () => {
    expect(stageSummary(Array.from({ length: 100 }, (_, i) => i + 1), 100)).toEqual({ p50: 50, p95: 95, max: 100 });
  });

  it('is zero for no samples', () => {
    expect(stageSummary([], 0)).toEqual({ p50: 0, p95: 0, max: 0 });
  });
});

describe('the per-minute line', () => {
  it('is nothing when no row arrived', () => {
    expect(takeChatLatencyLine()).toBeNull();
    startChatLatencyReporter();
    vi.advanceTimersByTime(180_000);
    expect(warn).not.toHaveBeenCalled();
  });

  it('reports each stage, the worst channel and the skew estimate', () => {
    // Twitch stamped at 1000, Rust read at 1100/1300, the page got them at 1150/1400, drawn at 1170/1500.
    noteRowReceived(row('a', 'quiet', 1000, 1100), 1150);
    noteRowReceived(row('b', 'busy', 1000, 1300), 1400);
    noteRowsPainted(['a'], 1170);
    noteRowsPainted(['b'], 1500);
    const line = takeChatLatencyLine();
    expect(line).toBe(
      '[ChatLatency] 2 rows, twitch->rust 100/300/300ms, rust->page 50/100/100ms, page->paint 20/100/100ms, worst busy 500ms, clock skew est 100ms',
    );
  });

  it('starts a new minute after each line', () => {
    noteRowReceived(row('a', 'c', 1000, 1100), 1150);
    expect(takeChatLatencyLine()).toContain('1 rows');
    expect(takeChatLatencyLine()).toBeNull();
  });

  it('logs at warn once a minute from the reporter', () => {
    startChatLatencyReporter();
    noteRowReceived(row('a', 'c', 1000, 1100), 1150);
    vi.advanceTimersByTime(60_000);
    expect(warn).toHaveBeenCalledTimes(1);
    expect(String(warn.mock.calls[0][0])).toMatch(/^\[ChatLatency\] 1 rows,/);
    vi.advanceTimersByTime(60_000);
    expect(warn).toHaveBeenCalledTimes(1);
  });

  it('skips the Rust stages for a row with no received_at_ms', () => {
    noteRowReceived(row('old', 'c', 1000), 1150);
    noteRowsPainted(['old'], 1200);
    expect(takeChatLatencyLine()).toBe(
      '[ChatLatency] 1 rows, twitch->rust n/a, rust->page n/a, page->paint 50/50/50ms, worst c 50ms, clock skew est n/a',
    );
  });

  it('ignores a frame with no id and a paint for an unknown id', () => {
    noteRowReceived({ channel: 'c' }, 10);
    noteRowsPainted(['nope'], 20);
    expect(takeChatLatencyLine()).toBeNull();
  });
});

describe('memory', () => {
  it('stays bounded at 10,000 rows a minute', () => {
    const ids: string[] = [];
    for (let i = 0; i < 10_000; i++) {
      const id = `r${i}`;
      ids.push(id);
      noteRowReceived(row(id, 'c', 1000, 1000 + (i % 50)), 1100 + (i % 70));
    }
    const held = latencyHeld();
    expect(held.twitchToRust).toBeLessThanOrEqual(512);
    expect(held.rustToPage).toBeLessThanOrEqual(512);
    expect(held.pending).toBeLessThanOrEqual(4096);
    noteRowsPainted(ids, 2000);
    expect(latencyHeld().pageToPaint).toBeLessThanOrEqual(512);
    expect(takeChatLatencyLine()).toContain('10000 rows');
    expect(latencyHeld().twitchToRust).toBe(0);
  });
});
