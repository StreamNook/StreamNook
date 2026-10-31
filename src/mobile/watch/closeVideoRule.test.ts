import { describe, expect, it } from 'vitest';
import {
  closeVideoRunner,
  closeVideoShown,
  miniCloseKeepsChat,
  runCloseVideo,
  tabIsStream,
  type CloseVideoDeps,
  type CloseVideoFacts,
} from './closeVideoRule';

const live: CloseVideoFacts = {
  mini: false,
  pip: false,
  onTV: false,
  streamUrl: 'http://127.0.0.1:9/live.m3u8',
  isLoading: false,
  mediaType: 'live',
  provider: 'twitch',
  userId: '42',
};

describe('closeVideoShown', () => {
  it('shows on a live stream in the full player, a rewound one and a failed one included', () => {
    expect(closeVideoShown(live)).toBe(true);
  });

  it.each([
    ['the mini box', { mini: true }],
    ['picture in picture', { pip: true }],
    ['a TV', { onTV: true }],
    ['a start still loading', { isLoading: true, streamUrl: null }],
    ['a Twitch row with no id yet', { userId: '' }],
    ['an offline room or an ended stream', { streamUrl: 'offline' }],
    ['a cast placeholder', { streamUrl: 'cast://xqc' }],
    ['a VOD', { mediaType: 'video' }],
    ['a clip', { mediaType: 'clip' }],
    ["an offline room's latest broadcast", { mediaType: 'offline_chat' }],
    ['nothing open', { streamUrl: null, mediaType: null }],
  ] as const)('hides in %s', (_name, patch) => {
    expect(closeVideoShown({ ...live, ...patch })).toBe(false);
  });

  it('shows for Kick, YouTube and TikTok, which carry no Twitch id', () => {
    for (const provider of ['kick', 'youtube', 'tiktok']) {
      expect(closeVideoShown({ ...live, provider, userId: '' })).toBe(true);
    }
  });
});

describe('tabIsStream', () => {
  it('matches the platform and the name, letter case aside', () => {
    expect(tabIsStream({ provider: 'twitch', login: 'xqc' }, { user_login: 'XQC' })).toBe(true);
    expect(tabIsStream({ provider: 'kick', login: 'xqc' }, { user_login: 'xqc', provider: 'kick' })).toBe(true);
  });

  it('never takes one platform for another or a missing stream for a match', () => {
    expect(tabIsStream({ provider: 'twitch', login: 'xqc' }, { user_login: 'xqc', provider: 'kick' })).toBe(false);
    expect(tabIsStream({ provider: 'kick', login: 'xqc' }, { user_login: 'xqc' })).toBe(false);
    expect(tabIsStream({ provider: 'twitch', login: 'xqc' }, null)).toBe(false);
  });
});

describe('miniCloseKeepsChat', () => {
  it('only on Chats with the stream room on screen', () => {
    const streamTab = { channel: 's' };
    expect(miniCloseKeepsChat({ onChats: true, activeChannel: 's', streamTab })).toBe(true);
    expect(miniCloseKeepsChat({ onChats: true, activeChannel: 'other', streamTab })).toBe(false);
    expect(miniCloseKeepsChat({ onChats: false, activeChannel: 's', streamTab })).toBe(false);
    expect(miniCloseKeepsChat({ onChats: true, activeChannel: 's', streamTab: null })).toBe(false);
  });
});

/** Fake steps that record what ran, in order. `afterKeep` is what plays once the
 *  keep answers (undefined: unchanged). */
function rig(opts: { tab?: { channel: string; kept: boolean } | null; keeps?: boolean; afterKeep?: string | null } = {}) {
  const calls: string[] = [];
  let playing: string | null = 'twitch:s';
  let release: () => void = () => {};
  let hold = false;
  const deps: CloseVideoDeps = {
    playing: () => playing,
    streamTab: () => (opts.tab === undefined ? { channel: 's', kept: false } : opts.tab),
    keep: async (channel) => {
      calls.push(`keep ${channel}`);
      if (hold) await new Promise<void>((r) => (release = r));
      if (opts.afterKeep !== undefined) playing = opts.afterKeep;
      return opts.keeps ?? true;
    },
    forget: (channel) => calls.push(`forget ${channel}`),
    toChats: () => calls.push('chats'),
    close: async () => {
      calls.push('close');
    },
    failed: () => calls.push('failed'),
  };
  return {
    deps,
    calls,
    holdKeep: () => (hold = true),
    unholdKeep: () => (hold = false),
    releaseKeep: () => release(),
    setPlaying: (p: string | null) => (playing = p),
  };
}

describe('runCloseVideo', () => {
  it('keeps the room, then Chats, then the close', async () => {
    const r = rig();
    await runCloseVideo(r.deps, true);
    expect(r.calls).toEqual(['keep s', 'chats', 'close']);
  });

  it('does nothing else until the keep has answered', async () => {
    const r = rig();
    r.holdKeep();
    const done = runCloseVideo(r.deps, true);
    await Promise.resolve();
    expect(r.calls).toEqual(['keep s']);
    r.releaseKeep();
    await done;
    expect(r.calls).toEqual(['keep s', 'chats', 'close']);
  });

  it('keeps a Kick room by the key Rust gave it', async () => {
    const r = rig({ tab: { channel: 'kick:xqc', kept: false } });
    await runCloseVideo(r.deps, true);
    expect(r.calls).toEqual(['keep kick:xqc', 'chats', 'close']);
  });

  it('the mini close on Chats keeps the room and closes, with no switch', async () => {
    const r = rig();
    await runCloseVideo(r.deps, false);
    expect(r.calls).toEqual(['keep s', 'close']);
  });

  it('a failed keep closes nothing and says so', async () => {
    const r = rig({ keeps: false });
    await runCloseVideo(r.deps, true);
    expect(r.calls).toEqual(['keep s', 'failed']);
  });

  it('a stream whose tab has not landed closes nothing and says so', async () => {
    const r = rig({ tab: null });
    await runCloseVideo(r.deps, true);
    expect(r.calls).toEqual(['failed']);
  });

  it('nothing playing does nothing', async () => {
    const r = rig();
    r.setPlaying(null);
    await runCloseVideo(r.deps, true);
    expect(r.calls).toEqual([]);
  });

  it('a newer stream during the keep is left playing, and a room that was not yours is let go', async () => {
    const r = rig({ afterKeep: 'twitch:other' });
    await runCloseVideo(r.deps, true);
    expect(r.calls).toEqual(['keep s', 'forget s']);
  });

  it('a newer stream during the keep leaves a room you already kept alone', async () => {
    const r = rig({ tab: { channel: 's', kept: true }, afterKeep: 'twitch:other' });
    await runCloseVideo(r.deps, true);
    expect(r.calls).toEqual(['keep s']);
  });

  it('a close by another route during the keep still lands on Chats', async () => {
    const r = rig({ afterKeep: null });
    await runCloseVideo(r.deps, true);
    expect(r.calls).toEqual(['keep s', 'chats']);
  });
});

describe('closeVideoRunner', () => {
  it('two presses close once, and a later press runs again', async () => {
    const r = rig();
    r.holdKeep();
    const close = closeVideoRunner(() => r.deps);
    const first = close(true);
    const second = close(true);
    expect(second).toBe(first);
    r.releaseKeep();
    await first;
    expect(r.calls.filter((c) => c.startsWith('keep'))).toHaveLength(1);
    r.unholdKeep();
    await close(true);
    expect(r.calls.filter((c) => c.startsWith('keep'))).toHaveLength(2);
  });
});
