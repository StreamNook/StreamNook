import { describe, it, expect } from 'vitest';
import {
  CHAT_BUFFER_SIZE,
  currentBufferLimit,
  liveAppendLimit,
  EVENT_LOOKBACK,
  EVENT_RETAIN,
  RESUME_DECAY_PER_FLUSH,
  isEventRow,
  resumeFlushLimit,
  resumeOverflowFor,
  trimWithEventRetention,
} from './chatBufferTrim';

const HISTORY_MAX = 100;
let seq = 0;
const chat = () => ({ id: `m${++seq}`, content: 'hi' });
const sub = () => ({ id: `e${++seq}`, metadata: { msg_type: 'resub', system_message: 'x resubscribed' } });

/** Simulates flushPending for an unpaused channel: one flush per batch of `perFlush` live rows. */
function runLive(messages: any[], flushes: number, perFlush: number, makeRow: () => any) {
  let liveCount = messages.length;
  let resumeOverflow = 0;
  let peak = messages.length;
  for (let f = 0; f < flushes; f++) {
    const next = resumeFlushLimit(HISTORY_MAX, resumeOverflow);
    resumeOverflow = next.resumeOverflow;
    for (let i = 0; i < perFlush; i++) messages.push(makeRow());
    liveCount += perFlush;
    messages = trimWithEventRetention(messages, next.limit, liveCount);
    peak = Math.max(peak, messages.length);
  }
  return { messages, peak };
}

describe('isEventRow', () => {
  it('recognises the event shapes and ignores plain rows and raw strings', () => {
    expect(isEventRow(sub())).toBe(true);
    expect(isEventRow({ tags: { 'custom-reward-id': 'abc' } })).toBe(true);
    expect(isEventRow({ tags: { 'msg-id': 'raid' } })).toBe(true);
    expect(isEventRow(chat())).toBe(false);
    expect(isEventRow('@badges=;id=1 :x!x@x PRIVMSG #c :hi')).toBe(false);
  });
});

describe('trimWithEventRetention', () => {
  it('keeps the last `limit` rows and rescues recent events from just before them', () => {
    const rows = [chat(), sub(), chat(), chat(), chat()];
    const out = trimWithEventRetention(rows, 2, 5);
    expect(out.map((r) => r.id)).toEqual([rows[1].id, rows[3].id, rows[4].id]);
  });

  it('never returns more than limit + EVENT_RETAIN rows', () => {
    const rows: any[] = [];
    for (let i = 0; i < 400; i++) rows.push(sub());
    expect(trimWithEventRetention(rows, HISTORY_MAX, 400).length).toBe(HISTORY_MAX + EVENT_RETAIN);
  });

  it('releases a rescued event after EVENT_LOOKBACK further live messages', () => {
    const pinned = sub();
    let messages: any[] = [pinned];
    for (let i = 0; i < HISTORY_MAX; i++) messages.push(chat());
    let liveCount = messages.length;
    // First trim rescues it and dates the rescue.
    messages.push(chat());
    liveCount++;
    messages = trimWithEventRetention(messages, HISTORY_MAX, liveCount);
    expect(messages[0]).toBe(pinned);
    // Still pinned while inside the lookback allowance.
    for (let i = 0; i < EVENT_LOOKBACK; i++) {
      messages.push(chat());
      liveCount++;
      messages = trimWithEventRetention(messages, HISTORY_MAX, liveCount);
    }
    expect(messages[0]).toBe(pinned);
    // One more message past the allowance and it scrolls off.
    messages.push(chat());
    liveCount++;
    messages = trimWithEventRetention(messages, HISTORY_MAX, liveCount);
    expect(messages.includes(pinned)).toBe(false);
    expect(messages.length).toBe(HISTORY_MAX);
  });
});

describe('unpaused live flushes stay bounded', () => {
  it('a channel with pinned events does not grow one row per flush (regression)', () => {
    // Six events at the head is exactly the shape that used to grow forever:
    // the old limit was max(historyMax, length - RESUME_DECAY_PER_FLUSH), so
    // each flush dropped 5 rows and rescued 6 back.
    const messages: any[] = [];
    for (let i = 0; i < 6; i++) messages.push(sub());
    for (let i = 0; i < HISTORY_MAX; i++) messages.push(chat());
    const { messages: out, peak } = runLive(messages, 2000, 1, chat);
    expect(peak).toBeLessThanOrEqual(HISTORY_MAX + EVENT_RETAIN);
    expect(out.length).toBeLessThanOrEqual(HISTORY_MAX + EVENT_RETAIN);
  });

  it('stays bounded under a mixed flood of events and chat', () => {
    const mixed = () => (seq % 4 === 0 ? sub() : chat());
    const { peak } = runLive([], 3000, 7, mixed);
    expect(peak).toBeLessThanOrEqual(HISTORY_MAX + EVENT_RETAIN);
  });
});

describe('resume decay', () => {
  it('records the paused overflow, capped at CHAT_BUFFER_SIZE', () => {
    expect(resumeOverflowFor(HISTORY_MAX + 40, HISTORY_MAX)).toBe(40);
    expect(resumeOverflowFor(HISTORY_MAX + 900, HISTORY_MAX)).toBe(CHAT_BUFFER_SIZE);
    expect(resumeOverflowFor(50, HISTORY_MAX)).toBe(0);
  });

  it('releases the overflow RESUME_DECAY_PER_FLUSH rows per flush down to the cap', () => {
    let overflow = CHAT_BUFFER_SIZE;
    const limits: number[] = [];
    for (let i = 0; i < 40; i++) {
      const next = resumeFlushLimit(HISTORY_MAX, overflow);
      overflow = next.resumeOverflow;
      limits.push(next.limit);
    }
    expect(limits[0]).toBe(HISTORY_MAX + CHAT_BUFFER_SIZE - RESUME_DECAY_PER_FLUSH);
    for (let i = 1; i < limits.length; i++) expect(limits[i]).toBeLessThanOrEqual(limits[i - 1]);
    expect(limits[limits.length - 1]).toBe(HISTORY_MAX);
  });

  it('a buffer that resumed at the paused cap shrinks back to the cap', () => {
    let messages: any[] = [];
    for (let i = 0; i < HISTORY_MAX + CHAT_BUFFER_SIZE; i++) messages.push(chat());
    let resumeOverflow = resumeOverflowFor(messages.length, HISTORY_MAX);
    let liveCount = messages.length;
    for (let f = 0; f < 60; f++) {
      const next = resumeFlushLimit(HISTORY_MAX, resumeOverflow);
      resumeOverflow = next.resumeOverflow;
      messages.push(chat());
      liveCount++;
      messages = trimWithEventRetention(messages, next.limit, liveCount);
    }
    expect(messages.length).toBe(HISTORY_MAX);
  });
});

/**
 * The store's own sequence, exercised through the shared policy: pause, let the
 * buffer grow into the cushion, resume, then keep appending. `kind` picks which
 * live path each step uses — a coalesced flush or a single direct push — because
 * the bug this covers was one path computing its own limit.
 */
function runResumeSequence(kind: 'flush' | 'push' | 'mixed') {
  const state = { isPausedForBuffer: false, resumeOverflow: 0 };
  let messages: any[] = [];
  let liveCount = 0;
  const append = (n: number) => {
    const limit = liveAppendLimit(state, HISTORY_MAX);
    for (let i = 0; i < n; i++) messages.push(chat());
    liveCount += n;
    messages = trimWithEventRetention(messages, limit, liveCount);
  };
  // Fill to the cap, then pause and grow into the cushion.
  append(HISTORY_MAX);
  state.isPausedForBuffer = true;
  for (let i = 0; i < CHAT_BUFFER_SIZE + 40; i++) append(1);
  const pausedLength = messages.length;
  // Resume exactly as setChannelPaused does.
  state.resumeOverflow = resumeOverflowFor(messages.length, HISTORY_MAX);
  state.isPausedForBuffer = false;
  const lengths: number[] = [];
  for (let step = 0; step < 60; step++) {
    append(kind === 'push' ? 1 : kind === 'flush' ? 3 : step % 2 === 0 ? 1 : 3);
    lengths.push(messages.length);
  }
  return { pausedLength, lengths, state };
}

describe('the live-append policy is shared by every path', () => {
  it('paused appends get the whole cushion and do not decay it', () => {
    const state = { isPausedForBuffer: true, resumeOverflow: CHAT_BUFFER_SIZE };
    expect(liveAppendLimit(state, HISTORY_MAX)).toBe(HISTORY_MAX + CHAT_BUFFER_SIZE);
    expect(state.resumeOverflow).toBe(CHAT_BUFFER_SIZE);
  });

  it.each(['flush', 'push', 'mixed'] as const)(
    'releases the cushion gradually after a resume driven by %s appends',
    (kind) => {
      const { pausedLength, lengths } = runResumeSequence(kind);
      expect(pausedLength).toBe(HISTORY_MAX + CHAT_BUFFER_SIZE);
      // Never a one-step collapse: the biggest single drop is the decay step.
      let prev = pausedLength;
      for (const len of lengths) {
        expect(prev - len).toBeLessThanOrEqual(RESUME_DECAY_PER_FLUSH);
        prev = len;
      }
      // ...and it does land back on the cap rather than hovering above it.
      expect(lengths[lengths.length - 1]).toBe(HISTORY_MAX);
    },
  );

  it('a direct push straight after a resume does not cut the cushion (regression)', () => {
    const state = { isPausedForBuffer: false, resumeOverflow: CHAT_BUFFER_SIZE };
    // The old pushMessage used a bare historyMax here, dropping 150 rows at once.
    expect(liveAppendLimit(state, HISTORY_MAX)).toBe(
      HISTORY_MAX + CHAT_BUFFER_SIZE - RESUME_DECAY_PER_FLUSH,
    );
  });
});

describe('currentBufferLimit', () => {
  it('reports the cushion without consuming it (history backfill path)', () => {
    const state = { isPausedForBuffer: false, resumeOverflow: 40 };
    expect(currentBufferLimit(state, HISTORY_MAX)).toBe(HISTORY_MAX + 40);
    expect(currentBufferLimit(state, HISTORY_MAX)).toBe(HISTORY_MAX + 40);
    expect(state.resumeOverflow).toBe(40);
  });

  it('gives a paused channel the full cushion and clamps a stale allowance', () => {
    expect(currentBufferLimit({ isPausedForBuffer: true, resumeOverflow: 0 }, HISTORY_MAX)).toBe(
      HISTORY_MAX + CHAT_BUFFER_SIZE,
    );
    expect(
      currentBufferLimit({ isPausedForBuffer: false, resumeOverflow: 9999 }, HISTORY_MAX),
    ).toBe(HISTORY_MAX + CHAT_BUFFER_SIZE);
  });
});
