import { describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import {
  openNoticeAction,
  readTaken,
  reportActivated,
  reportTaken,
  type NoticeDeps,
} from './noticeReport';
import type { NoticeAction } from './notificationMirror';

vi.mock('./platform', async (importOriginal) => ({
  ...(await importOriginal<typeof import('./platform')>()),
  IS_MAIN_WINDOW: true,
  isMainWindow: () => true,
}));

function harness(over: Partial<NoticeDeps> = {}) {
  const calls: string[] = [];
  const deps: NoticeDeps = {
    afterBoot: (fn) => fn(),
    currentStreamLogin: () => undefined,
    startStream: (channel) => void calls.push(`start:${channel}`),
    openWhisper: (user) => void calls.push(`whisper:${user.id}`),
    openDrops: () => void calls.push('drops'),
    openBadges: () => void calls.push('badges'),
    requestChangelog: (version) => void calls.push(`changelog:${version ?? ''}`),
    requestWallet: (channel) => void calls.push(`wallet:${channel}`),
    settingsTabAllowed: (tab) => tab === 'Notifications',
    openSettings: (tab) => void calls.push(`settings:${tab}`),
    openMention: (id, channel, opened) => {
      calls.push(`mention:${id}:${channel}`);
      opened();
      return Promise.resolve();
    },
    ...over,
  };
  const opened = () => void calls.push('opened');
  return { deps, calls, opened };
}

// A boot that has not ended: afterBoot parks the work until endBoot runs it.
function parkedBoot() {
  const parked: Array<() => void> = [];
  return {
    afterBoot: (fn: () => void) => void parked.push(fn),
    endBoot: () => parked.splice(0).forEach((fn) => fn()),
  };
}

describe('openNoticeAction calls opened once, where it opens', () => {
  // Fails without the change: a case that never calls `opened`, or calls it
  // before the open, gives a different list.
  it('whisper', () => {
    const { deps, calls, opened } = harness();
    openNoticeAction({ type: 'whisper', userId: '7', login: 'bob', name: 'Bob', avatar: null }, opened, deps);
    expect(calls).toEqual(['whisper:7', 'opened']);
  });

  it('whisper hands the store the user it expects', () => {
    const openWhisper = vi.fn();
    const { deps, opened } = harness({ openWhisper });
    openNoticeAction({ type: 'whisper', userId: '7', login: 'bob', name: 'Bob', avatar: 'a.png' }, opened, deps);
    expect(openWhisper.mock.calls[0][0]).toEqual({
      id: '7',
      login: 'bob',
      display_name: 'Bob',
      profile_image_url: 'a.png',
    });
  });

  it('drops, badges and changelog', () => {
    const a = harness();
    openNoticeAction({ type: 'drops' }, a.opened, a.deps);
    expect(a.calls).toEqual(['drops', 'opened']);
    const b = harness();
    openNoticeAction({ type: 'badges' }, b.opened, b.deps);
    expect(b.calls).toEqual(['badges', 'opened']);
    const c = harness();
    openNoticeAction({ type: 'changelog', version: '9.1.0' }, c.opened, c.deps);
    expect(c.calls).toEqual(['changelog:9.1.0', 'opened']);
    const d = harness();
    openNoticeAction({ type: 'changelog', version: null }, d.opened, d.deps);
    expect(d.calls).toEqual(['changelog:', 'opened']);
  });

  // Fails if `opened` is called before the allowed-tab check: the disallowed
  // tab would then report a screen that never opened.
  it('settings reports only for an allowed tab', () => {
    const ok = harness();
    openNoticeAction({ type: 'settings', tab: 'Notifications' }, ok.opened, ok.deps);
    expect(ok.calls).toEqual(['settings:Notifications', 'opened']);
    const no = harness();
    openNoticeAction({ type: 'settings', tab: 'Account' }, no.opened, no.deps);
    expect(no.calls).toEqual([]);
  });

  // Fails if the report is made when the click arrives instead of inside the
  // deferred start: the report would reach Rust while boot still holds the link.
  it('watch reports when its start runs, after boot when boot defers it', () => {
    const now = harness();
    openNoticeAction({ type: 'watch', channel: 'xqc' }, now.opened, now.deps);
    expect(now.calls).toEqual(['start:xqc', 'opened']);

    const boot = parkedBoot();
    const later = harness({ afterBoot: boot.afterBoot });
    openNoticeAction({ type: 'watch', channel: 'xqc' }, later.opened, later.deps);
    expect(later.calls).toEqual([]);
    boot.endBoot();
    expect(later.calls).toEqual(['start:xqc', 'opened']);
  });

  // Fails if the wallet-only branch skips `opened`, or reports for a channel
  // that was not the one playing before its start ran.
  it('rewards: wallet only when the channel is playing, a start otherwise', () => {
    const playing = harness({ currentStreamLogin: () => 'XQC' });
    openNoticeAction({ type: 'rewards', channel: 'xqc' }, playing.opened, playing.deps);
    expect(playing.calls).toEqual(['wallet:xqc', 'opened']);

    const other = harness({ currentStreamLogin: () => 'someone' });
    openNoticeAction({ type: 'rewards', channel: 'xqc' }, other.opened, other.deps);
    expect(other.calls).toEqual(['wallet:xqc', 'start:xqc', 'opened']);

    const boot = parkedBoot();
    const deferred = harness({ afterBoot: boot.afterBoot });
    openNoticeAction({ type: 'rewards', channel: 'xqc' }, deferred.opened, deferred.deps);
    expect(deferred.calls).toEqual(['wallet:xqc']);
    boot.endBoot();
    expect(deferred.calls).toEqual(['wallet:xqc', 'start:xqc', 'opened']);
  });

  // Fails if the mention case does not return the open's own promise (the take's
  // tracking would end at once), or if `opened` waits for the open: it must
  // have been called by the time openNoticeAction returns.
  it('mention returns its promise and has called opened before it settles', async () => {
    let finish!: () => void;
    const pending = new Promise<void>((resolveOpen) => { finish = resolveOpen; });
    const { deps, calls, opened } = harness({
      openMention: (id, channel, report) => {
        calls.push(`mention:${id}:${channel}`);
        report();
        return pending;
      },
    });
    const returned = openNoticeAction({ type: 'mention', id: 'm1', channel: 'xqc' }, opened, deps);
    expect(returned).toBe(pending);
    expect(calls).toEqual(['mention:m1:xqc', 'opened']);
    finish();
    await returned;
    expect(calls).toEqual(['mention:m1:xqc', 'opened']);
  });
});

describe('a notice click reports only what opened', () => {
  const run = (deps: NoticeDeps) => (action: NoticeAction, opened: () => void) =>
    openNoticeAction(action, opened, deps);

  // Fails if a disallowed settings tab reports (the click opened nothing, so a
  // link stored before it must survive).
  it('a disallowed settings tab never reports', () => {
    const { deps } = harness();
    const report = vi.fn();
    reportTaken({ action: { type: 'settings', tab: 'Account' }, seq: 9 }, run(deps), report);
    expect(report).not.toHaveBeenCalled();
  });

  it('an allowed settings tab reports its seq once', () => {
    const { deps } = harness();
    const report = vi.fn();
    reportTaken({ action: { type: 'settings', tab: 'Notifications' }, seq: 9 }, run(deps), report);
    expect(report).toHaveBeenCalledTimes(1);
    expect(report).toHaveBeenCalledWith(9);
  });

  it('a deferred watch reports only when boot ends', () => {
    const boot = parkedBoot();
    const { deps } = harness({ afterBoot: boot.afterBoot });
    const report = vi.fn();
    reportTaken({ action: { type: 'watch', channel: 'xqc' }, seq: 4 }, run(deps), report);
    expect(report).not.toHaveBeenCalled();
    boot.endBoot();
    expect(report).toHaveBeenCalledWith(4);
  });

  // Fails if reportTaken or the mention case drops the promise: the take's
  // tracking would end before the open does.
  it('the mention open reaches the caller as one promise', async () => {
    let finish!: () => void;
    const pending = new Promise<void>((resolveOpen) => { finish = resolveOpen; });
    const { deps } = harness({
      openMention: (_id, _channel, opened) => {
        opened();
        return pending;
      },
    });
    const report = vi.fn();
    const result = reportTaken({ action: { type: 'mention', id: 'm1', channel: 'xqc' }, seq: 2 }, run(deps), report);
    expect(result).toBe(pending);
    expect(report).toHaveBeenCalledWith(2);
    finish();
    await result;
  });

  // The activate listener: a click with no stored closure opens nothing.
  it('a click whose closure is missing reports nothing, one with a closure reports its seq', () => {
    const report = vi.fn();
    reportActivated({ id: 'gone', seq: 3 }, new Map(), report);
    expect(report).not.toHaveBeenCalled();

    const closure = vi.fn();
    reportActivated({ id: 'here', seq: 5 }, new Map([['here', closure]]), report);
    expect(closure).toHaveBeenCalledTimes(1);
    expect(report).toHaveBeenCalledTimes(1);
    expect(report).toHaveBeenCalledWith(5);
  });
});

describe('readTaken', () => {
  it('reads an action with its seq', () => {
    expect(readTaken({ action: { type: 'drops' }, seq: 6 })).toEqual({ action: { type: 'drops' }, seq: 6 });
  });

  // Each of these would run a bad payload as an action if the reader let it by.
  it('refuses no answer and malformed answers', () => {
    expect(readTaken(null)).toBeNull();
    expect(readTaken(undefined)).toBeNull();
    expect(readTaken('drops')).toBeNull();
    expect(readTaken({ action: { type: 'drops' } })).toBeNull();
    expect(readTaken({ action: { type: 'drops' }, seq: '6' })).toBeNull();
    expect(readTaken({ action: { type: 'drops' }, seq: Number.NaN })).toBeNull();
    expect(readTaken({ action: null, seq: 6 })).toBeNull();
    expect(readTaken({ action: { channel: 'xqc' }, seq: 6 })).toBeNull();
    expect(readTaken({ seq: 6 })).toBeNull();
  });
});

// Source pins: the component needs a DOM to mount, so its wiring is read as text.
const src = (p: string) => readFileSync(resolve(__dirname, '..', '..', p), 'utf8').replace(/\r\n/g, '\n');

describe('DynamicIsland uses the notice report helpers', () => {
  const text = src('src/components/DynamicIsland.tsx');

  it('runs actions through openNoticeAction and reads the take with readTaken', () => {
    expect(text).toContain('openNoticeAction(action, opened, NOTICE_DEPS)');
    expect(text).toContain('reportTaken(readTaken(taken), run, reportOpened)');
    expect(text).toContain("invoke<unknown>('take_notice_action')");
  });

  it('tracks the whole take and every report', () => {
    expect(text).toContain('trackNoticeWork(');
    expect(text).toContain("invoke('notice_opened', { seq })");
  });

  it('reads the activate event as { id, seq }', () => {
    expect(text).toContain('listen<{ id: string; seq: number }>(MIRRORED_NOTIFICATION_ACTIVATE_EVENT,');
    expect(text).toContain('reportActivated(event.payload, mirroredActionsRef.current, reportOpened)');
  });

  it('keeps the settings allow-list in the component and the Island preview reporting nothing', () => {
    expect(text).toContain("const SETTINGS_TABS_FROM_NOTICES: SettingsTab[] = ['Notifications'];");
    expect(text).toContain('settingsTabAllowed: (tab) => SETTINGS_TABS_FROM_NOTICES.includes(tab as SettingsTab)');
    expect(text).toContain('void openMentionInMain(entry.id, entry.open_key || entry.room);');
  });

  // Built from pieces so this file never contains the dead name itself.
  it('has no run switch of its own and never spells the superseded names', () => {
    expect(text).not.toContain('runNotice' + 'Action');
    expect(text).not.toContain('link_super' + 'seded');
    expect(src('src/keybindings/useKeybindings.ts')).not.toContain('link_super' + 'seded');
    expect(text).not.toContain("case 'rewards':");
  });
});
