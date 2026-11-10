// @vitest-environment node
import { describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));

const { managerState, blankManager, accountChanged, retryDelay, twitchAccountOf } = await import('./streamManager');
type State = Parameters<typeof managerState>[0];
type View = NonNullable<State['view']>;
type Queue = NonNullable<State['queue']>;

// The stream manager hook keeps one channel's copy, for one account: after a
// switch nothing of the channel you left shows, a late read, answer or push
// for it (or for another account) is dropped, and a read that started before
// a write never lands over the write's answer.

const view = (channel_id: string, rev = 1, user_id = 'u1') => ({ channel_id, channel_login: channel_id, rev, user_id }) as unknown as View;
const queue = (rev = 1, user_id = 'u1'): Queue => ({ groups: [], count: 0, empty_line: 'Nothing waiting', more: false, rev, user_id });
const start: State = { ...blankManager('a'), account: 'u1', view: view('a'), queue: queue(), viewRev: 1, queueRev: 1 };
const v = (channelId: string, vw: View, sent = 0) => ({ type: 'view' as const, channelId, view: vw, sent });

describe('managerState', () => {
  it('a switch forgets the old channel at once, but keeps the account and the revisions', () => {
    expect(managerState(start, { type: 'switch', channelId: 'b' })).toEqual({
      ...blankManager('b'),
      account: 'u1',
      viewRev: 1,
      queueRev: 1,
    });
    expect(managerState(start, { type: 'switch', channelId: 'a' })).toBe(start);
  });

  it('drops a late answer for the channel you left', () => {
    const b = managerState(start, { type: 'switch', channelId: 'b' });
    expect(managerState(b, v('a', view('a', 5)))).toBe(b);
    expect(managerState(b, { type: 'queue', channelId: 'a', queue: queue(5) })).toBe(b);
    expect(managerState(b, { type: 'error', channelId: 'a', error: 'x', seq: 9 })).toBe(b);
  });

  it('drops a view whose own channel is another one', () => {
    const b = managerState(start, { type: 'switch', channelId: 'b' });
    expect(managerState(b, v('b', view('a', 5)))).toBe(b);
  });

  it("takes the current channel's view and clears its error", () => {
    const failed = { ...start, error: 'Could not read the channel' };
    const next = managerState(failed, v('a', view('a', 2)));
    expect(next.error).toBeNull();
    expect(next.view?.rev).toBe(2);
  });

  it('with no channel, nothing lands', () => {
    const none = blankManager(null);
    expect(managerState(none, v('a', view('a')))).toBe(none);
  });
});

describe('revisions', () => {
  it('an older view never lands over a newer one; the same one still does', () => {
    const five = managerState(start, v('a', view('a', 5)));
    expect(five.viewRev).toBe(5);
    expect(managerState(five, v('a', view('a', 3)))).toBe(five);
    expect(managerState(five, v('a', view('a', 5))).view?.rev).toBe(5);
  });

  it('an older queue never lands over a newer one', () => {
    const four = managerState(start, { type: 'queue', channelId: 'a', queue: queue(4) });
    expect(managerState(four, { type: 'queue', channelId: 'a', queue: queue(3) })).toBe(four);
  });

  it('the revisions outlive a channel switch (Rust counts for the whole app)', () => {
    const five = managerState(start, v('a', view('a', 5)));
    const b = managerState(five, { type: 'switch', channelId: 'b' });
    expect(managerState(b, v('b', view('b', 4))).view).toBeNull();
    expect(managerState(b, v('b', view('b', 6))).view?.rev).toBe(6);
  });

  it('a failed read is dropped once a view from a later read has landed', () => {
    // Read 1 and read 2 are out; read 2's view lands, then read 1's failure.
    const landed = managerState(start, v('a', view('a', 2), 2));
    expect(managerState(landed, { type: 'error', channelId: 'a', error: 'late', seq: 1 })).toBe(landed);
    // A failure of a read sent after that view still shows.
    expect(managerState(landed, { type: 'error', channelId: 'a', error: 'now', seq: 3 }).error).toBe('now');
  });

  it('every failure counts, even with the same words, so retries go on', () => {
    const one = managerState(start, { type: 'error', channelId: 'a', error: 'x', seq: 1 });
    const two = managerState(one, { type: 'error', channelId: 'a', error: 'x', seq: 2 });
    expect(two.errors).toBe(one.errors + 1);
  });
});

describe('the account', () => {
  it("drops another account's view and queue", () => {
    expect(managerState(start, v('a', view('a', 5, 'u2')))).toBe(start);
    expect(managerState(start, { type: 'queue', channelId: 'a', queue: queue(5, 'u2') })).toBe(start);
  });

  it('another account blanks the view and keeps the revisions', () => {
    const next = managerState(start, { type: 'account', account: 'u2' });
    expect(next).toMatchObject({ account: 'u2', view: null, queue: null, viewRev: 1 });
  });

  it('becoming known keeps a view read as that account, and drops one read as another', () => {
    const unknown = { ...start, account: null };
    expect(managerState(unknown, { type: 'account', account: 'u1' }).view).toBe(start.view);
    expect(managerState(unknown, { type: 'account', account: 'u2' }).view).toBeNull();
  });

  it('is a change only between two known accounts, across a sign-out in between', () => {
    expect(accountChanged(null, 'a')).toBe(false);
    expect(accountChanged('a', 'a')).toBe(false);
    expect(accountChanged('a', null)).toBe(false);
    expect(accountChanged('a', 'b')).toBe(true);
  });

  it("reads the account from Rust's roster, only while connected", () => {
    const row = (status: string, account_id: string | null) => ({
      main: [{ id: 'twitch', status, account_id, name: null, handle: null, avatar_url: null }],
      extras: [],
    });
    expect(twitchAccountOf(null)).toBeNull();
    expect(twitchAccountOf(row('connected', '42') as never)).toBe('42');
    expect(twitchAccountOf(row('expired', '42') as never)).toBeNull();
  });
});

describe('retries', () => {
  it('backs off to every five minutes and keeps trying', () => {
    expect([0, 1, 2, 3, 10].map(retryDelay)).toEqual([5_000, 20_000, 60_000, 300_000, 300_000]);
  });
});
