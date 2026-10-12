// Run with: npm test
//
// The phone has no progress bar above its chat box, so how a /spam or
// /pyramid run ends arrives as one chat notice (watchBurstNotices).

import { beforeEach, describe, expect, it, vi } from 'vitest';

type Handler = (e: { payload: unknown }) => void;
const handlers: Handler[] = [];
const unlisten = vi.fn();
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (_event: string, h: Handler) => {
    handlers.push(h);
    return () => {
      unlisten();
      handlers.splice(handlers.indexOf(h), 1);
    };
  }),
}));
let running: unknown = { state: 'running' };
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => running) }));

import { burstLabel, watchBurstNotices, type BurstStatus } from './burstStatus';

const status = (patch: Partial<BurstStatus>): BurstStatus => ({
  channel: 'chan',
  kind: 'pyramid',
  sent: 5,
  total: 9,
  state: 'running',
  delay_ms: 1000,
  ...patch,
});
const fire = (s: BurstStatus) => [...handlers].forEach((h) => h({ payload: s }));

beforeEach(() => {
  handlers.length = 0;
  unlisten.mockReset();
  running = { state: 'running' };
});

describe('burst labels', () => {
  it('read like the desktop progress line', () => {
    expect(burstLabel(status({}))).toBe('Pyramid 5/9');
    expect(burstLabel(status({ state: 'done', sent: 9 }))).toBe('Pyramid done, 9/9');
    expect(burstLabel(status({ state: 'stopped' }))).toBe('Pyramid stopped at 5/9');
    expect(burstLabel(status({ state: 'failed', reason: 'Stopped: rate limited' }))).toBe('Stopped: rate limited');
  });
});

describe('watchBurstNotices', () => {
  it('prints how the run ended once, and only for its own channel', async () => {
    const notice = vi.fn();
    await watchBurstNotices('Chan', notice);
    fire(status({ sent: 6 }));
    fire(status({ channel: 'other', state: 'done' }));
    expect(notice).not.toHaveBeenCalled();
    fire(status({ state: 'failed', reason: 'Stopped: Twitch says you can\'t post here (msg_banned)' }));
    fire(status({ state: 'done' }));
    expect(notice.mock.calls).toEqual([["Stopped: Twitch says you can't post here (msg_banned)"]]);
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it('stops listening when no run is going any more', async () => {
    running = null;
    const notice = vi.fn();
    await watchBurstNotices('chan', notice);
    expect(unlisten).toHaveBeenCalledTimes(1);
    // A later run in the same channel is watched again.
    running = { state: 'running' };
    await watchBurstNotices('chan', notice);
    fire(status({ state: 'stopped' }));
    expect(notice).toHaveBeenCalledWith('Pyramid stopped at 5/9');
  });
});
