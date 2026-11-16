// The bridge wrappers the link doors call: each passes the activity's answer
// through, and falls back when the bridge is absent, lacks the method or throws.
import { afterEach, describe, expect, it } from 'vitest';
import {
  bootNowMs,
  consumePendingChannelIf,
  consumePendingMentionIf,
  consumePendingTap,
  consumePendingWhisperIf,
  isPipClosing,
  readPendingTap,
} from './nativeBridge';

const g = globalThis as Record<string, unknown>;
// `undefined` is a page with no bridge at all; an object is the bridge.
const withBridge = (bridge: Record<string, unknown> | undefined) => {
  g.window = bridge === undefined ? {} : { SNInsets: bridge };
};
const boom = () => {
  throw new Error('bridge gone');
};

afterEach(() => {
  delete g.window;
});

describe('consumePendingTap', () => {
  it('reads the tap the activity hands over', () => {
    withBridge({ consumePendingTap: () => '{"kind":"mention","value":"m1","at":1500}' });
    expect(consumePendingTap()).toEqual({ kind: 'mention', value: 'm1', at: 1500 });
  });

  it('is null for no tap, no bridge, a throw or unreadable text', () => {
    withBridge({ consumePendingTap: () => '' });
    expect(consumePendingTap()).toBeNull();
    withBridge(undefined);
    expect(consumePendingTap()).toBeNull();
    withBridge({ consumePendingTap: boom });
    expect(consumePendingTap()).toBeNull();
    withBridge({ consumePendingTap: () => '{oops' });
    expect(consumePendingTap()).toBeNull();
  });

  it('reads only the three kinds with a value, and a bad time as 0', () => {
    expect(readPendingTap({ kind: 'channel', value: 'xqc', at: 7 })).toEqual({ kind: 'channel', value: 'xqc', at: 7 });
    expect(readPendingTap({ kind: 'whisper', value: 'w', at: 'x' })).toEqual({ kind: 'whisper', value: 'w', at: 0 });
    expect(readPendingTap({ kind: 'mention', value: 'm', at: -3 })).toEqual({ kind: 'mention', value: 'm', at: 0 });
    expect(readPendingTap({ kind: 'badge', value: 'x', at: 1 })).toBeNull();
    expect(readPendingTap({ kind: 'channel', value: '', at: 1 })).toBeNull();
    expect(readPendingTap(null)).toBeNull();
  });
});

describe.each([
  ['consumePendingChannelIf', consumePendingChannelIf],
  ['consumePendingMentionIf', consumePendingMentionIf],
  ['consumePendingWhisperIf', consumePendingWhisperIf],
] as const)('%s', (name, wrapper) => {
  it('passes the time and the answer through', () => {
    const seen: number[] = [];
    withBridge({
      [name]: (at: number) => {
        seen.push(at);
        return at === 150;
      },
    });
    expect(wrapper(150)).toBe(true);
    expect(wrapper(100)).toBe(false);
    expect(seen).toEqual([150, 100]);
  });

  it('is true without a bridge: there is no stash to guard', () => {
    withBridge(undefined);
    expect(wrapper(150)).toBe(true);
  });

  it('is false for a bridge without the method, or one that throws', () => {
    withBridge({});
    expect(wrapper(150)).toBe(false);
    withBridge({ [name]: boom });
    expect(wrapper(150)).toBe(false);
  });
});

describe('bootNowMs and isPipClosing', () => {
  it('pass a value through', () => {
    withBridge({ bootNowMs: () => 86_400_000, isPipClosing: () => true });
    expect(bootNowMs()).toBe(86_400_000);
    expect(isPipClosing()).toBe(true);
  });

  it('answer 0 and false without a bridge or on a throw', () => {
    withBridge(undefined);
    expect(bootNowMs()).toBe(0);
    expect(isPipClosing()).toBe(false);
    withBridge({ bootNowMs: boom, isPipClosing: boom });
    expect(bootNowMs()).toBe(0);
    expect(isPipClosing()).toBe(false);
  });
});
