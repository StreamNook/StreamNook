// @vitest-environment node
import { describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));

const { managerState } = await import('./streamManager');
type State = Parameters<typeof managerState>[0];
type View = NonNullable<State['view']>;

// The stream manager hook keeps one channel's copy: after a switch nothing of
// the channel you left shows, and a late read, answer or push for it is
// dropped, so no form can write to it from here.

const view = (channel_id: string) => ({ channel_id, channel_login: channel_id }) as unknown as View;
const queue = { groups: [], count: 0, empty_line: 'Nothing waiting', more: false };
const start: State = { channelId: 'a', view: view('a'), queue, error: null };

describe('managerState', () => {
  it('a switch forgets the old channel at once', () => {
    expect(managerState(start, { type: 'switch', channelId: 'b' })).toEqual({ channelId: 'b', view: null, queue: null, error: null });
    expect(managerState(start, { type: 'switch', channelId: 'a' })).toBe(start);
  });

  it("drops a late answer for the channel you left", () => {
    const b = managerState(start, { type: 'switch', channelId: 'b' });
    expect(managerState(b, { type: 'view', channelId: 'a', view: view('a') })).toBe(b);
    expect(managerState(b, { type: 'queue', channelId: 'a', queue })).toBe(b);
    expect(managerState(b, { type: 'error', channelId: 'a', error: 'x' })).toBe(b);
  });

  it("drops a view whose own channel is another one", () => {
    const b = managerState(start, { type: 'switch', channelId: 'b' });
    expect(managerState(b, { type: 'view', channelId: 'b', view: view('a') })).toBe(b);
  });

  it("takes the current channel's view and clears its error", () => {
    const failed = { ...start, error: 'Could not read the channel' };
    const next = managerState(failed, { type: 'view', channelId: 'a', view: view('a') });
    expect(next.error).toBeNull();
    expect(next.view?.channel_id).toBe('a');
  });

  it('with no channel, nothing lands', () => {
    const none: State = { channelId: null, view: null, queue: null, error: null };
    expect(managerState(none, { type: 'view', channelId: 'a', view: view('a') })).toBe(none);
  });
});
