import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const core = vi.hoisted(() => ({ invoke: vi.fn() }));
const auth = vi.hoisted(() => ({ signedIn: true }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: core.invoke }));
vi.mock('../stores/AppStore', () => ({
  useAppStore: { getState: () => ({ isAuthenticated: auth.signedIn }) },
}));
vi.mock('./logger', () => ({ Logger: { debug: vi.fn(), info: vi.fn(), warn: vi.fn(), error: vi.fn() } }));

import {
  BADGE_LINK_LIMIT_MS,
  BADGE_LOAD_FAILED_TEXT,
  BADGE_NOT_FOUND_TEXT,
  BADGE_NOT_FOUND_TOAST,
  applyBadgeAnswer,
  findTwitchBadge,
  openBadgeLink,
  resolveBadgeLink,
  splitBadgeKey,
  type BadgeLinkAnswer,
  type BadgeLinkDeps,
} from './badgeLinkOpen';
import type { SelectedBadge } from '../components/BadgeDetailSlot';

// Every invoke waits for the test to answer it, so a test decides the order the reads land in.
interface Deferred {
  promise: Promise<unknown>;
  resolve: (value: unknown) => void;
  reject: (error: unknown) => void;
}
const asked: Record<string, Deferred> = {};
const calls: string[] = [];
const ask = (cmd: string): Deferred => {
  if (!asked[cmd]) {
    let resolveFn!: (value: unknown) => void;
    let rejectFn!: (error: unknown) => void;
    const promise = new Promise<unknown>((res, rej) => {
      resolveFn = res;
      rejectFn = rej;
    });
    asked[cmd] = { promise, resolve: resolveFn, reject: rejectFn };
  }
  return asked[cmd];
};
const settle = () => vi.advanceTimersByTimeAsync(0);

const version = (id: string) => ({
  id,
  image_url_1x: '1x',
  image_url_2x: '2x',
  image_url_4x: '4x',
  title: `Title ${id}`,
  description: '',
  click_action: null,
  click_url: null,
});
const list = (setId: string, ids: string[]) => ({ data: [{ set_id: setId, versions: ids.map(version) }] });

beforeEach(() => {
  vi.useFakeTimers();
  auth.signedIn = true;
  calls.length = 0;
  for (const cmd of Object.keys(asked)) delete asked[cmd];
  core.invoke.mockReset();
  core.invoke.mockImplementation((cmd: string) => {
    calls.push(cmd);
    return ask(cmd).promise;
  });
});

afterEach(() => {
  vi.useRealTimers();
});

const stubDeps = (over: Partial<BadgeLinkDeps> = {}, log: string[] = []): BadgeLinkDeps => ({
  isPlayerFullscreen: () => false,
  exitPlayerFullscreen: () => void log.push('exit'),
  showBadgeLink: (key) => {
    log.push(`show:${key}`);
    return 7;
  },
  answerBadgeLink: (key, read, next) => void log.push(`answer:${key}#${read}:${next.kind === 'badge' ? 'badge' : next.state}`),
  ...over,
});

describe('openBadgeLink', () => {
  // Fails if the detail waits for the lookup: show must be logged before any read has been answered.
  it('shows the placeholder at once, before any read answers', () => {
    const log: string[] = [];
    openBadgeLink({ setId: 'clover', version: '2' }, stubDeps({}, log));
    expect(log).toEqual(['show:clover/2']);
    expect(calls).toEqual(['get_cached_global_badges']);
  });

  // Fails if the answer carries a different read number than showBadgeLink returned.
  it('hands the lookup answer back with the key and the read number it was started for', async () => {
    const log: string[] = [];
    openBadgeLink({ setId: 'clover', version: '2' }, stubDeps({}, log));
    ask('get_cached_global_badges').resolve(list('clover', ['1', '2']));
    await settle();
    expect(log).toEqual(['show:clover/2', 'answer:clover/2#7:badge']);
  });

  it('leaves player full screen first, only when the player is in it', () => {
    const log: string[] = [];
    openBadgeLink({ setId: 'a', version: '1' }, stubDeps({ isPlayerFullscreen: () => true }, log));
    expect(log).toEqual(['exit', 'show:a/1']);
  });

  it('does not touch full screen otherwise', () => {
    const exit = vi.fn();
    openBadgeLink({ setId: 'a', version: '1' }, stubDeps({ exitPlayerFullscreen: exit }));
    expect(exit).not.toHaveBeenCalled();
  });
});

describe('resolveBadgeLink', () => {
  const answers = (): BadgeLinkAnswer[] => {
    const seen: BadgeLinkAnswer[] = [];
    resolveBadgeLink('clover/2', (next) => seen.push(next));
    return seen;
  };

  // Fails if a cached hit still waits for the forced refresh.
  it('answers a cached hit without the refresh, carrying the set id Twitch spells', async () => {
    const seen = answers();
    ask('get_cached_global_badges').resolve(list('Clover', ['2']));
    await settle();
    expect(seen).toEqual([{ kind: 'badge', badge: { ...version('2'), set_id: 'Clover' }, setId: 'Clover' }]);
    expect(calls).toEqual(['get_cached_global_badges']);
  });

  it('refreshes once on a cache miss and answers the hit', async () => {
    const seen = answers();
    ask('get_cached_global_badges').resolve(list('clover', ['1']));
    await settle();
    expect(seen).toEqual([]);
    expect(calls).toEqual(['get_cached_global_badges', 'force_refresh_global_badges']);
    ask('force_refresh_global_badges').resolve(list('clover', ['1', '2']));
    await settle();
    expect(seen).toEqual([{ kind: 'badge', badge: { ...version('2'), set_id: 'clover' }, setId: 'clover' }]);
  });

  // Fails if a read that completed with no match is called failed (or the other way round).
  it('answers missing when both reads completed and neither has the badge', async () => {
    const seen = answers();
    ask('get_cached_global_badges').resolve(list('clover', ['1']));
    await settle();
    ask('force_refresh_global_badges').resolve(list('clover', ['1']));
    await settle();
    expect(seen).toEqual([{ kind: 'link', key: 'clover/2', state: 'missing' }]);
  });

  // Fails if an offline read is told as "not found".
  it('answers failed when the refresh is rejected, whether or not a cached list exists', async () => {
    const withList = answers();
    ask('get_cached_global_badges').resolve(list('clover', ['1']));
    await settle();
    ask('force_refresh_global_badges').reject(new Error('offline'));
    await settle();
    expect(withList).toEqual([{ kind: 'link', key: 'clover/2', state: 'failed' }]);

    for (const cmd of Object.keys(asked)) delete asked[cmd];
    const noList = answers();
    ask('get_cached_global_badges').resolve(null);
    await settle();
    ask('force_refresh_global_badges').reject(new Error('offline'));
    await settle();
    expect(noList).toEqual([{ kind: 'link', key: 'clover/2', state: 'failed' }]);
  });

  it('still tries the refresh when the cached read itself is rejected', async () => {
    const seen = answers();
    ask('get_cached_global_badges').reject(new Error('cache'));
    await settle();
    ask('force_refresh_global_badges').resolve(list('clover', ['2']));
    await settle();
    expect(seen.map((a) => a.kind)).toEqual(['badge']);
  });

  // Fails if signed out still forces a refresh (it needs a token) or says "not found" with no list at all.
  it('signed out, answers from the cached list alone and never forces a refresh', async () => {
    auth.signedIn = false;
    const loaded = answers();
    ask('get_cached_global_badges').resolve(list('clover', ['1']));
    await settle();
    expect(loaded).toEqual([{ kind: 'link', key: 'clover/2', state: 'missing' }]);

    for (const cmd of Object.keys(asked)) delete asked[cmd];
    const none = answers();
    ask('get_cached_global_badges').resolve(null);
    await settle();
    expect(none).toEqual([{ kind: 'link', key: 'clover/2', state: 'failed' }]);
    expect(calls.filter((c) => c === 'force_refresh_global_badges')).toEqual([]);
  });

  it('signed out, a cached hit still answers the badge', async () => {
    auth.signedIn = false;
    const seen = answers();
    ask('get_cached_global_badges').resolve(list('clover', ['2']));
    await settle();
    expect(seen.map((a) => a.kind)).toEqual(['badge']);
  });

  // Fails if the timer cancels the read or the call settles once: the 8 s hit would never reach onAnswer.
  it('answers failed at 6 s without canceling the read, then answers the read whenever it lands', async () => {
    const seen = answers();
    ask('get_cached_global_badges').resolve(list('clover', ['1']));
    await settle();
    await vi.advanceTimersByTimeAsync(BADGE_LINK_LIMIT_MS - 1);
    expect(seen).toEqual([]);
    await vi.advanceTimersByTimeAsync(1);
    expect(seen).toEqual([{ kind: 'link', key: 'clover/2', state: 'failed' }]);
    await vi.advanceTimersByTimeAsync(2000);
    ask('force_refresh_global_badges').resolve(list('clover', ['2']));
    await settle();
    expect(seen.map((a) => a.kind === 'link' ? a.state : a.kind)).toEqual(['failed', 'badge']);
  });

  // Fails if the timer is left running after the read answered: a stray failed would follow the badge.
  it('stops the timer once the read has answered', async () => {
    const seen = answers();
    ask('get_cached_global_badges').resolve(list('clover', ['2']));
    await settle();
    await vi.advanceTimersByTimeAsync(BADGE_LINK_LIMIT_MS * 2);
    expect(seen.map((a) => a.kind)).toEqual(['badge']);
  });
});

describe('splitBadgeKey', () => {
  it('splits at the last slash', () => {
    expect(splitBadgeKey('clover/2')).toEqual({ setId: 'clover', version: '2' });
    expect(splitBadgeKey('a/b/3')).toEqual({ setId: 'a/b', version: '3' });
    expect(splitBadgeKey('solo')).toEqual({ setId: 'solo', version: '' });
  });
});

describe('applyBadgeAnswer', () => {
  const loading: SelectedBadge = { kind: 'link', key: 'clover/2', state: 'loading', read: 3 };
  const failed: SelectedBadge = { kind: 'link', key: 'clover/2', state: 'failed', read: 3 };
  const hit: BadgeLinkAnswer = { kind: 'badge', badge: version('2'), setId: 'clover' };
  const miss: BadgeLinkAnswer = { kind: 'link', key: 'clover/2', state: 'missing' };
  const fail: BadgeLinkAnswer = { kind: 'link', key: 'clover/2', state: 'failed' };

  it('a loading placeholder takes any answer for its key and read, and a link answer keeps the read', () => {
    expect(applyBadgeAnswer(loading, 'clover/2', 3, hit)).toEqual({ kind: 'badge', badge: version('2'), setId: 'clover' });
    expect(applyBadgeAnswer(loading, 'clover/2', 3, miss)).toEqual({ kind: 'link', key: 'clover/2', state: 'missing', read: 3 });
    expect(applyBadgeAnswer(loading, 'clover/2', 3, fail)).toEqual({ kind: 'link', key: 'clover/2', state: 'failed', read: 3 });
  });

  // Fails if a late hit cannot fill a placeholder the 6 s timer already failed.
  it('a failed placeholder takes a late hit, and nothing else', () => {
    expect(applyBadgeAnswer(failed, 'clover/2', 3, hit)).toEqual({ kind: 'badge', badge: version('2'), setId: 'clover' });
    expect(applyBadgeAnswer(failed, 'clover/2', 3, miss)).toBe(failed);
    expect(applyBadgeAnswer(failed, 'clover/2', 3, fail)).toBe(failed);
  });

  // Fails if an answer for another key or an older read can overwrite the placeholder the person sees.
  it('an answer for another key or an older read writes nothing', () => {
    expect(applyBadgeAnswer(loading, 'other/1', 3, hit)).toBe(loading);
    expect(applyBadgeAnswer(loading, 'clover/2', 2, hit)).toBe(loading);
    const retry: SelectedBadge = { kind: 'link', key: 'clover/2', state: 'loading', read: 4 };
    expect(applyBadgeAnswer(retry, 'clover/2', 3, hit)).toBe(retry);
  });

  // Fails if a retry's link answer drops the Try again mark, or a plain read's answer gains one.
  it('a link answer keeps the slot\'s Try again mark and adds none', () => {
    const retrying: SelectedBadge = { kind: 'link', key: 'clover/2', state: 'loading', read: 3, retry: true };
    expect(applyBadgeAnswer(retrying, 'clover/2', 3, fail)).toStrictEqual({ kind: 'link', key: 'clover/2', state: 'failed', read: 3, retry: true });
    expect(applyBadgeAnswer(loading, 'clover/2', 3, fail)).toStrictEqual({ kind: 'link', key: 'clover/2', state: 'failed', read: 3 });
  });

  // Fails if a closed or replaced slot is reopened by a late answer.
  it('writes nothing when the slot was closed or already shows a badge', () => {
    const shown: SelectedBadge = { kind: 'badge', badge: version('9'), setId: 'other' };
    expect(applyBadgeAnswer(null, 'clover/2', 3, hit)).toBeNull();
    expect(applyBadgeAnswer(shown, 'clover/2', 3, hit)).toBe(shown);
  });
});

describe('findTwitchBadge', () => {
  const sets = [
    { set_id: 'clover', versions: [{ id: '1', title: 'one' }, { id: '2', title: 'two' }] },
    { set_id: 'Payday-3', versions: [{ id: '1', title: 'pd' }] },
  ];

  it('finds the exact version and carries its set id', () => {
    expect(findTwitchBadge(sets, 'clover', '2')).toEqual({ id: '2', title: 'two', set_id: 'clover' });
  });

  it('compares the set id without case', () => {
    expect(findTwitchBadge(sets, 'payday-3', '1')?.set_id).toBe('Payday-3');
  });

  it('compares the version exactly', () => {
    expect(findTwitchBadge(sets, 'clover', '3')).toBeUndefined();
    expect(findTwitchBadge(sets, 'clover', '')).toBeUndefined();
  });

  it('misses an unknown set', () => {
    expect(findTwitchBadge(sets, 'nope', '1')).toBeUndefined();
  });
});

describe('the sheet words', () => {
  it('stay neutral about who is at fault', () => {
    expect(BADGE_NOT_FOUND_TEXT).toBe("Couldn't find this badge yet");
    expect(BADGE_LOAD_FAILED_TEXT).toBe("Couldn't load this badge");
  });

  // Android's RewardsScreen still imports the old name until its own slice deletes the alias.
  it('keeps the old toast name for Android', () => {
    expect(BADGE_NOT_FOUND_TOAST).toBe(BADGE_NOT_FOUND_TEXT);
  });
});

// Source pins: behavior that lives in components and needs a DOM to mount.
const src = (p: string) => readFileSync(resolve(__dirname, '..', '..', p), 'utf8').replace(/\r\n/g, '\n');

describe('shells use the one link door', () => {
  it('desktop takes links through one drain on take_pending_link', () => {
    const text = src('src/App.tsx');
    expect(text.match(/startLinkDrain\(/g)).toHaveLength(1);
    expect(text).toContain("take: () => invoke<unknown>('take_pending_link')");
    expect(text).toContain('ready: () => !useAppStore.getState().isBooting');
    expect(new Set(text.match(/take_pending_\w+/g))).toEqual(new Set(['take_pending_link']));
    expect(text).toContain('drainRef.current?.stop();');
  });

  it('desktop feeds the boot watch and waits for notice work before a take', () => {
    const text = src('src/App.tsx');
    expect(text).toContain('beforeTake: noticeWorkSettled');
    expect(text).toContain('onStamp: noteLinkStamp');
    expect(text).toContain("peek: () => trackLinkPeek(invoke<unknown>('peek_pending_link_stamp'))");
  });

  it('desktop takes again once boot ends and watches the person\'s actions while it boots', () => {
    const text = src('src/App.tsx');
    const at = text.indexOf('drainRef.current?.drain()');
    expect(at).toBeGreaterThan(-1);
    const start = text.lastIndexOf('useEffect(', at);
    const end = text.indexOf('}, [isBooting]);', at);
    expect(start).toBeGreaterThan(-1);
    expect(end).toBeGreaterThan(at);
    const effect = text.slice(start, end);
    expect(effect).toMatch(/if \(!isBooting\) \{\s*drainRef\.current\?\.drain\(\);/);
    expect(effect).toContain('return watchBootActions(useAppStore, invoke);');
    expect(effect.indexOf('drainRef.current?.drain()')).toBeLessThan(effect.indexOf('watchBootActions('));
  });

  it('desktop holds no link in the webview and closes its own layers through one function', () => {
    const text = src('src/App.tsx');
    expect(text).not.toMatch(/holdLink|takeHeld/);
    expect(text).not.toContain('handleChangelogCloseRef');
    expect(text).toContain('makeCloseAppLayers({ changelogRecordsSeenRef, setShowChangelog, setSelectedBadge })');
  });

  it('phone takes links through the doors on take_pending_link, after the playback listeners', () => {
    const text = src('src/mobile/boot/useMobileBoot.ts');
    expect(text.match(/doors\.mountDrain\(/g)).toHaveLength(1);
    expect(text).toMatch(/addCleanup\(\s*doors\.mountDrain\(/);
    expect(text).toContain("take: () => invoke<unknown>('take_pending_link')");
    expect(new Set(text.match(/take_pending_\w+/g))).toEqual(new Set(['take_pending_link']));
    expect(text).toContain("import { listen } from '@tauri-apps/api/event';");
    expect(text).not.toMatch(/listen:\s*addListener/);
    expect(text).toContain('tap: consumePendingTap(),');
    // Built from halves: the absence scan over src/ must never find the old bridge's name in this file.
    expect(text).not.toContain('startPhoneLink' + 'Bridge(');
    // Fails if the shell spells deps of its own instead of the ones the tests drive.
    expect(text).toContain('const linkDeps = makePhoneLinkDeps({ openChannel });');
    expect(text).not.toMatch(/:\s*InboundLinkDeps\s*=/);
    expect(text).toContain('onLink: (link) => openInboundLink(link, linkDeps),');
    // Fails if the shell holds by a rule the tests do not drive.
    expect(text).toContain('shouldHold: phoneShouldHold,');
    expect(text).toContain('bootNow: bootNowMs,');
    // Fails if a link can start a stream before the playback listeners exist, or a torn-down boot still drains.
    const order = [
      "'provider-stream-offline'",
      'if (!isMounted) return;',
      'const doors = createPhoneLinkDoors({',
      'addCleanup(installHeldLinkRelease(doors.release));',
      'doors.mountDrain(',
      'addCleanup(installWarmTaps(doors));',
    ];
    let at = -1;
    for (const needle of order) {
      const found = text.indexOf(needle, at + 1);
      expect(found, needle).toBeGreaterThan(at);
      at = found;
    }
    // The block between the guard and the warm taps has no await: no gap for a warm tap with no handler.
    const block = text.slice(text.indexOf('if (!isMounted) return;'), text.indexOf('addCleanup(installWarmTaps(doors));'));
    expect(block).not.toMatch(/\bawait\b/);
  });
});

describe('App badge slot', () => {
  const app = src('src/App.tsx');

  // Fails if App mounts a badge overlay of its own again, outside the slot the render tests cover.
  it('renders the slot and mounts no badge overlay itself', () => {
    expect(app).toContain('<BadgeDetailSlot');
    expect(app).not.toContain('<BadgesOverlay');
    expect(app).not.toContain('<BadgeDetailOverlay');
  });

  // Fails if App spells a close of its own: the render tests then cover a copy, not the code App runs.
  it('builds its closeLinkSheet with makeCloseLinkSheet and spells no close body of its own', () => {
    expect(app).toContain('makeCloseLinkSheet({ setSelectedBadge, setShowBadgesOverlay })');
    expect(app).not.toContain('useBadgeSpotlightStore');
  });

  // Fails if any writer of the slot leaves the kind tag off.
  it('every writer of selectedBadge writes a kind', () => {
    const writers = [
      ...app.matchAll(/setSelectedBadge\(\{[^}]*\}\)/g),
      ...src('src/components/BadgeDetailSlot.tsx').matchAll(/setSelectedBadge\(\{[^}]*\}\)/g),
    ].map((m) => m[0]);
    expect(writers.length).toBeGreaterThanOrEqual(4);
    for (const writer of writers) expect(writer).toContain('kind:');
  });

  it('answers through the pure rule and numbers every read from one counter', () => {
    expect(app).toContain('applyBadgeAnswer(cur, key, read, next)');
    expect(app.match(/badgeReadRef\.current \+= 1/g)).toHaveLength(2);
  });
});

describe('BadgesOverlay holds no link code', () => {
  const text = src('src/components/BadgesOverlay.tsx');

  // Positive control: the needle scan reads the right file.
  it('still opens a tile through onBadgeClick and keeps its own refresh button', () => {
    expect(text).toContain('onBadgeClick(badge, badge.set_id)');
    expect(text).toContain("'force_refresh_global_badges'");
  });

  // Fails if any piece of the old in-overlay link effect comes back. The word the widened absence pin
  // looks for is built from two halves so this file never spells it.
  it.each([
    'consumeInitial' + 'TwitchBadge',
    'badgesOverlayInitial' + 'TwitchBadge',
    'useIsPresent',
    'linkMountedRef',
    'presentRef',
    'BADGE_NOT_FOUND',
    'findTwitchBadge',
    'link' + 'Gen',
  ])('has no %s', (needle) => {
    expect(text).not.toContain(needle);
  });
});

describe('phone sheets all listen for sn:dismiss-sheet', () => {
  it.each([
    'src/mobile/ui/MobileSheet.tsx',
    'src/mobile/rewards/RewardsPopover.tsx',
    'src/mobile/ui/PlatformMenu.tsx',
  ])('%s', (file) => {
    const text = src(file);
    expect(text).toContain("addEventListener('sn:dismiss-sheet'");
    expect(text).toContain("removeEventListener('sn:dismiss-sheet'");
  });

  it('MobileSheet dismisses with onClose, not its back step', () => {
    const text = src('src/mobile/ui/MobileSheet.tsx');
    const at = text.indexOf('const onDismiss');
    expect(text.slice(at, at + 160)).toContain('onClose()');
    expect(text.slice(at, at + 160)).not.toContain('back()');
  });

  it('Back still steps with the string sn:close-sheet', () => {
    expect(src('src/mobile/navStore.ts')).toContain("new CustomEvent('sn:close-sheet', { detail: top })");
  });
});

describe('RewardsScreen link read', () => {
  const text = src('src/mobile/screens/RewardsScreen.tsx');

  // Fails if the link forces a network read before trying the cache, or reads through the gallery store.
  it('reads the cached wall first and forces one refresh only on a miss, never through the store', () => {
    const cached = text.indexOf('cached = await getBadgeWall(sort(), false)');
    const refresh = text.indexOf("await invoke('force_refresh_global_badges')");
    expect(cached).toBeGreaterThan(-1);
    expect(refresh).toBeGreaterThan(cached);
    expect(text).not.toContain('getBadgeWall(badgeSortRef.current, true)');
    expect(text).not.toContain('loadBadgeGallery(badgeSortRef.current, true)');
  });

  it('follows with a plain read at the sort current at answer time', () => {
    expect(text).toContain('void loadBadgeGallery(badgeSortRef.current, false)');
  });

  it('takes its key from the render closure, never from getState', () => {
    expect(text).not.toMatch(/getState\(\)\.badgeLinkKey/);
    expect(text).toContain('const key = badgeLinkKey;');
  });

  // Fails if the link effect gains a cleanup (it would cancel its own read) or shows a toast again.
  it('the link effect has no cleanup and no toast', () => {
    const start = text.indexOf('if (!badgeLinkKey) return;');
    const end = text.indexOf('}, [badgeLinkKey, startLinkRead]);');
    expect(start).toBeGreaterThan(-1);
    expect(end).toBeGreaterThan(start);
    expect(text.slice(start, end)).not.toMatch(/return \(\) =>/);
    expect(text).not.toContain('addToast(BADGE_NOT_FOUND');
  });

  it('answers through the pure rule', () => {
    expect(text).toContain('applyBadgeAnswer(cur, key, read, next)');
  });
});
