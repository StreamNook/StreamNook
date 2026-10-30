import { describe, expect, it } from 'vitest';
import { LIVE_ALERT_INITIAL, liveAlertReduce, shownFor, type LiveAlertView } from './liveAlertView';

const K = 'twitch|1|alpha';

function run(events: Parameters<typeof liveAlertReduce>[1][]): LiveAlertView {
  return events.reduce(liveAlertReduce, LIVE_ALERT_INITIAL);
}

describe('live alert bell view', () => {
  it('shows the first answer', () => {
    const v = run([{ kind: 'answer', key: K, id: 1, on: true }]);
    expect(shownFor(v, K)).toBe(true);
    expect(v.busy).toBe(false);
  });

  it('drops a read that started before a press and answered after it', () => {
    const v = run([
      { kind: 'press', key: K, id: 2, on: false },
      { kind: 'answer', key: K, id: 1, on: true },
    ]);
    expect(shownFor(v, K)).toBe(false);
    expect(v.busy).toBe(true);
  });

  it('takes the press answer and stops being busy', () => {
    const v = run([
      { kind: 'answer', key: K, id: 1, on: true },
      { kind: 'press', key: K, id: 2, on: false },
      { kind: 'answer', key: K, id: 2, on: false },
    ]);
    expect(shownFor(v, K)).toBe(false);
    expect(v.busy).toBe(false);
  });

  it('a newer read wins over an older press answer that lands late', () => {
    const v = run([
      { kind: 'press', key: K, id: 2, on: false },
      { kind: 'answer', key: K, id: 3, on: false },
      { kind: 'answer', key: K, id: 2, on: true },
    ]);
    expect(shownFor(v, K)).toBe(false);
  });

  it('a failed press goes back to the last answer', () => {
    const v = run([
      { kind: 'answer', key: K, id: 1, on: true },
      { kind: 'press', key: K, id: 2, on: false },
      { kind: 'failed', key: K, id: 2 },
    ]);
    expect(shownFor(v, K)).toBe(true);
    expect(v.busy).toBe(false);
  });

  it('shows nothing for another channel until it answers', () => {
    const v = run([{ kind: 'answer', key: K, id: 1, on: true }]);
    expect(shownFor(v, 'kick|2|beta')).toBeNull();
  });

  it('keeps a no-identity answer as null so the bell hides', () => {
    const v = run([{ kind: 'answer', key: K, id: 1, on: null }]);
    expect(shownFor(v, K)).toBeNull();
  });
});
