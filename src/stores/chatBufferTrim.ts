/**
 * Chat buffer capping, kept pure so it can be unit-tested without the store.
 *
 * Every chat row (plain chat, subs, gift bombs, redemptions, raids) shares one
 * capped buffer per channel. Two rules shape the cap:
 *
 * 1. Event retention: a burst of low-value rows (a mass-gift's children, a
 *    sub-bot posting one line per sub, plain spam) must not evict the recent
 *    high-value events with it. `trimWithEventRetention` keeps the last `limit`
 *    rows AND rescues up to EVENT_RETAIN event rows from just before that
 *    window. A rescued row is re-pinned on every later trim until
 *    EVENT_LOOKBACK further live messages have arrived; after that it scrolls
 *    off like anything else. The buffer therefore never exceeds
 *    `limit + EVENT_RETAIN`.
 *
 * 2. Resume decay: while the user is scrolled up the buffer may hold
 *    CHAT_BUFFER_SIZE rows above the cap. On resume that overflow is released a
 *    few rows per flush (`resumeFlushLimit`) instead of in one cut, so the
 *    scrollback the user just read does not vanish mid-glide. The allowance is
 *    tracked as its own number and decays independently of the buffer length:
 *    deriving it from `messages.length` let rescued event rows feed back into
 *    the limit, and a channel with six or more pinned events then grew by one
 *    row per flush for the rest of the session (measured: 100 rows to 1,150 in
 *    six minutes on a busy channel, DOM nodes 3k to 29k, JS heap 33 MB to 80 MB).
 */

/** Extra rows allowed while a channel is paused (scrolled up). */
export const CHAT_BUFFER_SIZE = 150;
/** Rows of post-resume overflow released per flush. */
export const RESUME_DECAY_PER_FLUSH = 5;
/** Max event rows rescued from the drop region on one trim. */
export const EVENT_RETAIN = 30;
/**
 * How many further live messages an event row survives past its natural
 * scroll-off before it stops being rescued. Also bounds how far back one trim
 * scans for rescue candidates.
 */
export const EVENT_LOOKBACK = 600;

const EVENT_MSG_IDS = new Set([
  'sub', 'resub', 'subgift', 'submysterygift', 'anonsubgift', 'anonsubmysterygift',
  'raid', 'unraid', 'viewermilestone', 'announcement', 'bitsbadgetier', 'charitydonation',
  'highlighted-message', 'gigantified-emote-message', 'animated-message', 'skip-subs-mode-message',
]);

export function isEventRow(m: any): boolean {
  if (!m || typeof m !== 'object') return false; // raw-string fallback rows aren't rescued
  const mt = m.metadata?.msg_type || m.tags?.['msg-id'];
  return !!(
    m.metadata?.system_message ||
    m.tags?.['system-msg'] ||
    m.tags?.['custom-reward-id'] ||
    (mt && EVENT_MSG_IDS.has(mt))
  );
}

/**
 * Live-message count at which a row was first rescued. Keyed by the row
 * object itself, so nothing is written onto message objects and the entry
 * disappears with the row.
 */
const rescueBase = new WeakMap<object, number>();

/**
 * Keep the last `limit` rows plus up to EVENT_RETAIN recent event rows from
 * just before that window. `liveCount` is the channel's monotonic live message
 * counter; it dates each rescue so a row is released after EVENT_LOOKBACK more
 * messages. Pass `undefined` for paths without one (backfill), which rescues
 * without dating.
 */
export function trimWithEventRetention(messages: any[], limit: number, liveCount?: number): any[] {
  if (messages.length <= limit) return messages;
  const windowStart = messages.length - limit;
  const recentTail = messages.slice(windowStart);
  const rescued: any[] = [];
  const lookbackStart = Math.max(0, windowStart - EVENT_LOOKBACK);
  for (let i = windowStart - 1; i >= lookbackStart && rescued.length < EVENT_RETAIN; i--) {
    const m = messages[i];
    if (!isEventRow(m)) continue;
    if (typeof liveCount === 'number') {
      const base = rescueBase.get(m);
      if (base === undefined) rescueBase.set(m, liveCount);
      else if (liveCount - base > EVENT_LOOKBACK) continue; // had its extra time; let it go
    }
    rescued.push(m);
  }
  if (rescued.length === 0) return recentTail;
  rescued.reverse(); // newest-first scan back to chronological order
  return rescued.concat(recentTail);
}

/** The slice fields this module reads and updates. Kept structural so the
 *  policy can be exercised without constructing a whole store slice. */
export interface ChatBufferState {
  isPausedForBuffer: boolean;
  resumeOverflow: number;
}

/**
 * The cap for one live append, whether that is a coalesced rAF flush or a
 * single direct push (own sends, NOTICEs, redemptions). Paused: the full
 * scrollback cushion. Not paused: the cap plus whatever post-resume allowance
 * is left, decayed one step as a side effect — so every live path drains the
 * cushion at the same gentle rate. A path that computed its own limit instead
 * (the old `pushMessage`) cut the whole cushion in one step, which is the
 * visible jump the decay exists to avoid.
 */
export function liveAppendLimit(state: ChatBufferState, historyMax: number): number {
  if (state.isPausedForBuffer) return historyMax + CHAT_BUFFER_SIZE;
  const next = resumeFlushLimit(historyMax, state.resumeOverflow);
  state.resumeOverflow = next.resumeOverflow;
  return next.limit;
}

/**
 * Allowed buffer length right now, WITHOUT decaying the allowance. For paths
 * that trim but are not a live append — history backfill prepends a block and
 * then cuts to the cap, and doing that against a bare `historyMax` while the
 * user still has a resume cushion drops up to CHAT_BUFFER_SIZE rows in one
 * step: the same visible jump the decay exists to avoid.
 */
export function currentBufferLimit(state: ChatBufferState, historyMax: number): number {
  const cushion = state.isPausedForBuffer
    ? CHAT_BUFFER_SIZE
    : Math.max(0, Math.min(CHAT_BUFFER_SIZE, state.resumeOverflow));
  return historyMax + cushion;
}

/** Overflow allowance to record when a channel resumes from paused. */
export function resumeOverflowFor(bufferLength: number, historyMax: number): number {
  return Math.max(0, Math.min(CHAT_BUFFER_SIZE, bufferLength - historyMax));
}

/**
 * The cap for one flush of a channel that is not paused. Decays the remaining
 * post-resume allowance by RESUME_DECAY_PER_FLUSH and returns the limit plus
 * the allowance left for next time. Always at least `historyMax`, never more
 * than `historyMax + CHAT_BUFFER_SIZE`.
 */
export function resumeFlushLimit(
  historyMax: number,
  resumeOverflow: number,
): { limit: number; resumeOverflow: number } {
  const remaining = Math.max(0, Math.min(CHAT_BUFFER_SIZE, resumeOverflow) - RESUME_DECAY_PER_FLUSH);
  return { limit: historyMax + remaining, resumeOverflow: remaining };
}
