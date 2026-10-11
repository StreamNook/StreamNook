import { describe, expect, it, vi } from 'vitest';

vi.mock('../stores/chatConnectionStore', () => ({
  useChatConnectionStore: { getState: () => ({ channels: new Map() }) },
}));

import { resolvePinArgs } from './pinCommand';

const ID = '0f8a2c4e-1b3d-4e5f-8a9b-0c1d2e3f4a5b';
const recent: Record<string, string> = { bob: 'bob-msg', alice: 'alice-msg' };
const latestFrom = (login: string) => recent[login.toLowerCase()] ?? null;
const resolve = (text: string) => resolvePinArgs(text.split(/\s+/), latestFrom);

describe('/pin argument resolution', () => {
  it('pins a message id, with or without a duration', () => {
    expect(resolve(ID)).toEqual({ kind: 'message', messageId: ID, durationSeconds: null });
    expect(resolve(`${ID} 5m`)).toEqual({ kind: 'message', messageId: ID, durationSeconds: 300 });
  });

  it("pins a chatter's latest message by @name or bare name", () => {
    expect(resolve('@bob')).toEqual({ kind: 'message', messageId: 'bob-msg', durationSeconds: null });
    expect(resolve('Alice 90s')).toEqual({ kind: 'message', messageId: 'alice-msg', durationSeconds: 90 });
  });

  it('says so when an @name has no recent message', () => {
    expect(resolve('@carol')).toEqual({ error: 'No recent message from carol in this chat' });
  });

  it('sends and pins anything else, keeping a trailing duration as text', () => {
    expect(resolve('welcome to the stream')).toEqual({ kind: 'send', text: 'welcome to the stream' });
    expect(resolve('hello')).toEqual({ kind: 'send', text: 'hello' });
    expect(resolve('back in 5m')).toEqual({ kind: 'send', text: 'back in 5m' });
    expect(resolve('bob is on fire')).toEqual({ kind: 'send', text: 'bob is on fire' });
    expect(resolve('@bob nice play')).toEqual({ kind: 'send', text: '@bob nice play' });
  });

  it('needs something to work with', () => {
    expect(resolvePinArgs([], latestFrom)).toHaveProperty('error');
    expect(resolvePinArgs([''], latestFrom)).toHaveProperty('error');
  });
});
