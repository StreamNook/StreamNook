// The composer's @ list is ranked in Rust (`mention_suggestions`, the streamer
// first). This checks the desktop bridge asks that question and keeps Rust's
// order; the ranking itself is proven by emote_match's Rust tests.
import { beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
  convertFileSrc: (p: string) => `asset://${p}`,
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }));

import { matchMentions } from './emoteMatch';

const target = { provider: 'twitch' as const, channel: 'kaileymae', channelId: '9' };

describe('mention suggestions', () => {
  beforeEach(() => invoke.mockReset());

  it('asks Rust and keeps its order', async () => {
    invoke.mockResolvedValue([
      { login: 'kaileymae', user_id: '9', display: 'KaileyMae' },
      { login: 'knutzz', user_id: '1', display: '' },
    ]);
    const rows = await matchMentions(target, 'k');
    expect(invoke).toHaveBeenCalledWith('mention_suggestions', {
      provider: 'twitch',
      channel: 'kaileymae',
      channelId: '9',
      query: 'k',
      commandArg: false,
    });
    expect(rows).toEqual([
      { login: 'kaileymae', userId: '9', displayName: 'KaileyMae' },
      { login: 'knutzz', userId: '1', displayName: 'knutzz' },
    ]);
  });

  it('says when the query is a command argument', async () => {
    invoke.mockResolvedValue([]);
    await matchMentions(target, 'k', true);
    expect(invoke).toHaveBeenCalledWith('mention_suggestions', expect.objectContaining({ commandArg: true }));
  });

  it('asks nothing without a channel', async () => {
    expect(await matchMentions({ ...target, channel: '' }, 'k')).toEqual([]);
    expect(invoke).not.toHaveBeenCalled();
  });

  it('answers empty when Rust fails', async () => {
    // Only the mention question fails; anything else the module asks still answers.
    invoke.mockImplementation((cmd: string) =>
      cmd === 'mention_suggestions' ? Promise.reject(new Error('denied')) : Promise.resolve(undefined),
    );
    expect(await matchMentions(target, 'k')).toEqual([]);
  });
});
