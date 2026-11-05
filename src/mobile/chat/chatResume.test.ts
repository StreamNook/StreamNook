// The return from the background joins first, waits for the joins, then
// rebuilds after a long absence (chatResume.ts).
import { describe, it, expect, vi, beforeEach } from 'vitest';

const order: string[] = [];
let releaseConnect: () => void = () => {};
const connect = vi.fn(
  () =>
    new Promise<void>((r) => {
      releaseConnect = () => {
        order.push('connect');
        r();
      };
    }),
);
let releaseSettle: () => void = () => {};
const settleJoins = vi.fn(
  () =>
    new Promise<void>((r) => {
      releaseSettle = () => {
        order.push('settle');
        r();
      };
    }),
);
const hardCycleChat = vi.fn(async () => {
  order.push('rebuild');
  return true;
});

vi.mock('./chatTabsStore', () => ({
  useChatTabsStore: { getState: () => ({ connect }) },
  settleJoins: (...a: unknown[]) => settleJoins(...(a as [])),
}));
vi.mock('./chatRecovery', () => ({ hardCycleChat: (...a: unknown[]) => hardCycleChat(...(a as [])) }));

import { resumeChats, CHAT_REBUILD_AFTER_HIDDEN_MS, RESUME_JOIN_WAIT_MS } from './chatResume';

beforeEach(() => {
  order.length = 0;
  connect.mockClear();
  settleJoins.mockClear();
  hardCycleChat.mockClear();
});

const flush = async () => {
  for (let i = 0; i < 5; i += 1) await Promise.resolve();
};

describe('resumeChats', () => {
  it('joins, waits for the joins, then rebuilds after a long absence', async () => {
    const done = resumeChats(CHAT_REBUILD_AFTER_HIDDEN_MS + 1_000);
    await flush();
    expect(connect).toHaveBeenCalledWith({ resume: true });
    expect(settleJoins).not.toHaveBeenCalled();
    releaseConnect();
    await flush();
    expect(settleJoins).toHaveBeenCalledTimes(1);
    expect(hardCycleChat).not.toHaveBeenCalled();
    releaseSettle();
    await done;
    expect(order).toEqual(['connect', 'settle', 'rebuild']);
    expect(hardCycleChat).toHaveBeenCalledWith('away for 91s');
  });

  it('a short absence joins but never rebuilds', async () => {
    const done = resumeChats(30_000);
    releaseConnect();
    await flush();
    releaseSettle();
    await done;
    expect(order).toEqual(['connect', 'settle']);
  });

  it('a join that never answers still lets the rebuild run, within the one deadline', async () => {
    vi.useFakeTimers();
    try {
      const done = resumeChats(CHAT_REBUILD_AFTER_HIDDEN_MS + 1_000);
      await vi.advanceTimersByTimeAsync(RESUME_JOIN_WAIT_MS);
      expect(settleJoins).toHaveBeenCalledWith(0);
      releaseSettle();
      await done;
      expect(hardCycleChat).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });
});
