// The phone shell's link pieces, driven as useMobileBoot wires them: the real
// opener deps and nav store behind the doors, the hold rule, the release
// listeners and the warm tap handlers over a fake activity stash.
import { beforeEach, describe, expect, it, vi } from 'vitest';

const env = vi.hoisted(() => {
  type Event = { type: string; detail?: unknown };
  type Handler = (e: Event) => void;
  const target = () => {
    const handlers = new Map<string, Set<Handler>>();
    return {
      addEventListener: (type: string, handler: Handler) => {
        const set = handlers.get(type) ?? new Set<Handler>();
        set.add(handler);
        handlers.set(type, set);
      },
      removeEventListener: (type: string, handler: Handler) => {
        handlers.get(type)?.delete(handler);
      },
      dispatchEvent: (e: Event) => {
        for (const handler of [...(handlers.get(e.type) ?? [])]) handler(e);
        return true;
      },
    };
  };
  const win = target();
  const doc = Object.assign(target(), { visibilityState: 'visible' });
  const g = globalThis as Record<string, unknown>;
  g.window = win;
  g.document = doc;
  g.CustomEvent = class {
    constructor(
      public type: string,
      init?: { detail?: unknown },
    ) {
      this.detail = init?.detail;
    }
    detail: unknown;
  };
  return { win, doc, log: [] as string[] };
});

const native = vi.hoisted(() => {
  type Stash = { value: string; at: number } | null;
  return {
    pip: false as boolean | null,
    closing: false,
    stash: { channel: null as Stash, mention: null as Stash, whisper: null as Stash },
  };
});

const spotlight = vi.hoisted(() => ({ close: vi.fn() }));

vi.mock('../../stores/AppStore', () => ({
  useAppStore: { getState: () => ({ streamUrl: null, isLoading: false }) },
}));
vi.mock('../../stores/badgeSpotlightStore', () => ({
  useBadgeSpotlightStore: { getState: () => ({ close: spotlight.close }) },
}));
vi.mock('../phonePrefs', () => ({ startTab: () => 'following' }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => null) }));
vi.mock('../../utils/logger', () => ({ Logger: { debug: vi.fn(), info: vi.fn(), warn: vi.fn(), error: vi.fn() } }));
vi.mock('../nativeBridge', () => {
  const consumeIf = (kind: 'channel' | 'mention' | 'whisper') => (at: number) => {
    const stashed = native.stash[kind];
    if (stashed && stashed.at === at) {
      native.stash[kind] = null;
      return true;
    }
    return false;
  };
  return {
    isInPip: () => native.pip,
    isPipClosing: () => native.closing,
    consumePendingChannelIf: consumeIf('channel'),
    consumePendingMentionIf: consumeIf('mention'),
    consumePendingWhisperIf: consumeIf('whisper'),
  };
});

import { installHeldLinkRelease, installWarmTaps, makePhoneLinkDeps, phoneShouldHold } from './phoneLinkDeps';
import { useMobileNavStore } from '../navStore';
import {
  clearHeldLink,
  createPhoneLinkDoors,
  openInboundLink,
  takeHeld,
  type InboundLinkTarget,
} from '../../utils/inboundLinkOpen';

const flush = () => new Promise<void>((done) => setTimeout(done, 0));
const watch: InboundLinkTarget = { kind: 'watch', key: 'xqc' };
const clover: InboundLinkTarget = { kind: 'badge', setId: 'clover', version: '3' };

beforeEach(() => {
  env.log.length = 0;
  env.doc.visibilityState = 'visible';
  native.pip = false;
  native.closing = false;
  native.stash = { channel: null, mention: null, whisper: null };
  spotlight.close.mockClear();
  clearHeldLink();
  useMobileNavStore.setState({
    activeTab: 'following',
    sheetStack: [],
    badgeLinkKey: null,
    browseCategory: null,
    linkSheetSeq: 0,
    playerMode: 'full',
  });
});

// The phone as useMobileBoot builds it: real deps, real doors, a held flag the test flips.
function phone() {
  const hold = { value: false };
  const openChannel = vi.fn((key: string) => void env.log.push(`stream:${key}`));
  const linkDeps = makePhoneLinkDeps({ openChannel });
  const doors = createPhoneLinkDoors({
    shouldHold: () => hold.value,
    openMention: vi.fn(),
    openWhisper: vi.fn(),
    openChannel,
    onLink: (link) => openInboundLink(link, linkDeps),
    bootNow: () => 0,
  });
  return { hold, openChannel, linkDeps, doors };
}

function withSheetOpen(id: string) {
  useMobileNavStore.setState({ sheetStack: [id] });
  const record = (e: { detail?: unknown }) => void env.log.push(`dismiss:${String(e.detail)}`);
  env.win.addEventListener('sn:dismiss-sheet', record);
  return () => env.win.removeEventListener('sn:dismiss-sheet', record);
}

describe('makePhoneLinkDeps', () => {
  // Fails if the two-argument door is wired straight to the one-key store action.
  it('opens a badge link under its one key and a category link nameless on Browse', () => {
    const { linkDeps } = phone();
    openInboundLink(clover, linkDeps);
    expect(useMobileNavStore.getState().badgeLinkKey).toBe('clover/3');
    openInboundLink({ kind: 'category', provider: 'twitch', id: '509658' }, linkDeps);
    expect(useMobileNavStore.getState().browseCategory).toEqual({ id: '509658', name: '', box_art_url: '' });
    expect(useMobileNavStore.getState().activeTab).toBe('browse');
  });
});

describe('a watch link closes the sheet a link opened, before the stream starts', () => {
  // Fails if the doors' watch arm starts the stream directly, skipping closeLinkSheet.
  it('through doors.onLink', () => {
    const stop = withSheetOpen('sheet-1');
    phone().doors.onLink(watch, 5);
    expect(env.log).toEqual(['dismiss:sheet-1', 'stream:xqc']);
    expect(spotlight.close).toHaveBeenCalled();
    expect(useMobileNavStore.getState().badgeLinkKey).toBeNull();
    stop();
  });

  it("through the mount's take", async () => {
    const stop = withSheetOpen('sheet-1');
    const cleanup = phone().doors.mountDrain({
      tap: null,
      listen: async () => () => {},
      take: async () => ({ link: watch, at: 5 }),
    });
    await flush();
    expect(env.log).toEqual(['dismiss:sheet-1', 'stream:xqc']);
    cleanup();
    stop();
  });

  it('after a held badge link was released', () => {
    const p = phone();
    p.hold.value = true;
    p.doors.onLink(clover, 10);
    expect(useMobileNavStore.getState().badgeLinkKey).toBeNull();
    p.hold.value = false;
    p.doors.release();
    expect(useMobileNavStore.getState().badgeLinkKey).toBe('clover/3');
    const stop = withSheetOpen('sheet-2');
    p.doors.onLink(watch, 20);
    expect(env.log).toEqual(['dismiss:sheet-2', 'stream:xqc']);
    expect(useMobileNavStore.getState().badgeLinkKey).toBeNull();
    expect(takeHeld()).toBeNull();
    stop();
  });
});

describe('phoneShouldHold', () => {
  it('holds in picture in picture or while hidden, and not otherwise', () => {
    native.pip = true;
    expect(phoneShouldHold()).toBe(true);
    native.pip = false;
    expect(phoneShouldHold()).toBe(false);
    native.pip = null;
    env.doc.visibilityState = 'hidden';
    expect(phoneShouldHold()).toBe(true);
  });
});

describe('installHeldLinkRelease', () => {
  // Fails if a dismissed PiP window releases at once, or sn:pip-closed releases on its own.
  it('releases on every visibility change and on sn:pip unless the window was dismissed', () => {
    const release = vi.fn();
    const cleanup = installHeldLinkRelease(release);
    env.doc.dispatchEvent({ type: 'visibilitychange' });
    expect(release).toHaveBeenCalledTimes(1);
    env.win.dispatchEvent({ type: 'sn:pip', detail: false });
    expect(release).toHaveBeenCalledTimes(2);
    native.closing = true;
    env.win.dispatchEvent({ type: 'sn:pip', detail: false });
    env.win.dispatchEvent({ type: 'sn:pip-closed' });
    expect(release).toHaveBeenCalledTimes(2);
    native.closing = false;
    cleanup();
    env.doc.dispatchEvent({ type: 'visibilitychange' });
    env.win.dispatchEvent({ type: 'sn:pip', detail: false });
    expect(release).toHaveBeenCalledTimes(2);
  });
});

describe('installWarmTaps', () => {
  type Handlers = {
    __SN_OPEN_CHANNEL__?: (value: string, at?: number) => void;
    __SN_OPEN_MENTION__?: (value: string, at?: number) => void;
    __SN_OPEN_WHISPER__?: (value: string, at?: number) => void;
  };
  const w = () => env.win as unknown as Handlers;
  const doorsSpy = () => ({ channelTap: vi.fn(), mentionTap: vi.fn(), whisperTap: vi.fn() });

  // Fails if a warm tap leaves its stash behind (a later mount would open it again).
  it('takes its own stash, then hands the value and its time to the door', () => {
    const doors = doorsSpy();
    const cleanup = installWarmTaps(doors);
    native.stash.channel = { value: 'xqc', at: 150 };
    w().__SN_OPEN_CHANNEL__?.('xqc', 150);
    expect(native.stash.channel).toBeNull();
    expect(doors.channelTap).toHaveBeenCalledWith('xqc', 150);
    native.stash.mention = { value: 'm1', at: 151 };
    w().__SN_OPEN_MENTION__?.('m1', 151);
    expect(doors.mentionTap).toHaveBeenCalledWith('m1', 151);
    native.stash.whisper = { value: 'w1', at: 152 };
    w().__SN_OPEN_WHISPER__?.('w1', 152);
    expect(doors.whisperTap).toHaveBeenCalledWith('w1', 152);
    cleanup();
    expect(w().__SN_OPEN_CHANNEL__).toBeUndefined();
  });

  // Fails if a hand-over opens a tap the mount already opened.
  it('opens nothing for a tap the mount already took', () => {
    const p = phone();
    const cleanup = installWarmTaps(p.doors);
    native.stash.channel = { value: 'xqc', at: 150 };
    p.doors.mountTap({ kind: 'channel', value: 'xqc', at: 150 });
    native.stash.channel = null;
    w().__SN_OPEN_CHANNEL__?.('xqc', 150);
    expect(p.openChannel).toHaveBeenCalledTimes(1);
    cleanup();
  });

  // Fails if an older hand-over clears a newer tap's stash or opens over it.
  it('an older hand-over leaves a newer stash for its own hand-over', () => {
    const doors = doorsSpy();
    const cleanup = installWarmTaps(doors);
    native.stash.channel = { value: 'c2', at: 150 };
    w().__SN_OPEN_CHANNEL__?.('c1', 100);
    expect(doors.channelTap).not.toHaveBeenCalled();
    expect(native.stash.channel).toEqual({ value: 'c2', at: 150 });
    w().__SN_OPEN_CHANNEL__?.('c2', 150);
    expect(doors.channelTap).toHaveBeenCalledWith('c2', 150);
    cleanup();
  });

  it('ignores an empty value', () => {
    const doors = doorsSpy();
    const cleanup = installWarmTaps(doors);
    native.stash.channel = { value: 'xqc', at: 0 };
    w().__SN_OPEN_CHANNEL__?.('', 0);
    expect(native.stash.channel).not.toBeNull();
    expect(doors.channelTap).not.toHaveBeenCalled();
    cleanup();
  });
});
