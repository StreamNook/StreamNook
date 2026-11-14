import { describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import {
  BADGE_NOT_FOUND_TOAST,
  findTwitchBadge,
  openBadgeLink,
  readBadgeLink,
  type BadgeLinkDeps,
} from './badgeLinkOpen';

const stubDeps = (over: Partial<BadgeLinkDeps> = {}, calls: string[] = []): BadgeLinkDeps => ({
  isPlayerFullscreen: () => false,
  exitPlayerFullscreen: () => void calls.push('exit'),
  closeBadgeDetail: () => void calls.push('close'),
  openTwitchBadge: (setId, version) => void calls.push(`open:${setId}/${version}`),
  ...over,
});

describe('openBadgeLink', () => {
  it('closes an open detail, then opens the Twitch badge', () => {
    const calls: string[] = [];
    openBadgeLink({ setId: 'clover', version: '2' }, stubDeps({}, calls));
    expect(calls).toEqual(['close', 'open:clover/2']);
  });

  it('leaves player full screen first, only when the player is in it', () => {
    const calls: string[] = [];
    openBadgeLink({ setId: 'a', version: '1' }, stubDeps({ isPlayerFullscreen: () => true }, calls));
    expect(calls).toEqual(['exit', 'close', 'open:a/1']);
  });

  it('does not touch full screen otherwise', () => {
    const exit = vi.fn();
    openBadgeLink({ setId: 'a', version: '1' }, stubDeps({ exitPlayerFullscreen: exit }));
    expect(exit).not.toHaveBeenCalled();
  });
});

describe('readBadgeLink', () => {
  it('reads what Rust returns', () => {
    expect(readBadgeLink({ setId: 'clover', version: '3' })).toEqual({ setId: 'clover', version: '3' });
  });

  it('defaults a missing version and rejects non-links', () => {
    expect(readBadgeLink({ setId: 'clover' })).toEqual({ setId: 'clover', version: '1' });
    expect(readBadgeLink(null)).toBeNull();
    expect(readBadgeLink({ setId: '' })).toBeNull();
    expect(readBadgeLink('clover')).toBeNull();
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

describe('the toast words', () => {
  it('stay neutral about who is at fault', () => {
    expect(BADGE_NOT_FOUND_TOAST).toBe("Couldn't find this badge yet");
  });
});

// Source pins: behavior that lives in components and needs a DOM to mount.
const src = (p: string) => readFileSync(resolve(__dirname, '..', '..', p), 'utf8').replace(/\r\n/g, '\n');

describe('shells listen and drain', () => {
  it.each(['src/App.tsx', 'src/mobile/boot/useMobileBoot.ts'])('%s', (file) => {
    const text = src(file);
    expect(text).toMatch(/streamnook:badge'/);
    expect(text).toMatch(/invoke<unknown>\('take_pending_badge_link'\)/);
  });

  it('desktop opens through openBadgeLink and holds a link while booting', () => {
    const text = src('src/App.tsx');
    expect(text).toContain('openBadgeLink(link, {');
    expect(text).toContain('pendingBadgeLinkRef.current = link');
    expect(text).toContain('closeBadgeDetail: () => setSelectedBadge(null)');
  });

  it('phone opens through navStore.openBadgeLink', () => {
    expect(src('src/mobile/boot/useMobileBoot.ts')).toContain('openBadgeLink(`${link.setId}/${link.version}`)');
  });
});

describe('BadgesOverlay link handling', () => {
  const text = src('src/components/BadgesOverlay.tsx');
  const effect = text.slice(text.indexOf('const link = useAppStore.getState().consumeInitialTwitchBadge()'));

  it('consumes the target before any onBadgeClick', () => {
    const consume = text.indexOf('consumeInitialTwitchBadge()');
    expect(consume).toBeGreaterThan(-1);
    expect(consume).toBeLessThan(text.indexOf('onBadgeClick(hit, hit.set_id)'));
    expect(consume).toBeLessThan(text.indexOf('onBadgeClick(hitAfterRefresh'));
  });

  it('refreshes straight from Rust, not through forceRefreshBadges', () => {
    const end = effect.indexOf('}, [loading, error, badgesWithMetadata, pendingTwitchBadge])');
    expect(end).toBeGreaterThan(-1);
    const refresh = effect.slice(0, end);
    expect(refresh).toContain("invoke<{ data: BadgeSet[] }>('force_refresh_global_badges')");
    expect(refresh).not.toContain('forceRefreshBadges');
  });

  it('runs again for a link that lands while the overlay is already open', () => {
    // Subscribed to the store field, and the field is in the effect's deps:
    // reading it only through getState() would leave the link waiting for an
    // unrelated remount (then it would reopen on Back).
    expect(text).toContain('const pendingTwitchBadge = useAppStore((s) => s.badgesOverlayInitialTwitchBadge);');
    expect(text).toContain('if (loading || error || !pendingTwitchBadge) return;');
    expect(text).toContain('}, [loading, error, badgesWithMetadata, pendingTwitchBadge]);');
  });

  it('drops a stale answer: newer link, unmounted, overlay closed, or exiting', () => {
    expect(effect).toContain('linkGen.current !== gen');
    expect(effect).toContain('!linkMountedRef.current');
    expect(effect).toContain('!useAppStore.getState().showBadgesOverlay');
    expect(effect).toContain('!presentRef.current');
    expect(text).toContain('linkMountedRef.current = true');
    expect(text).toContain('presentRef.current = isPresent');
  });

  it('says the neutral words on a second miss', () => {
    expect(effect).toContain("addToast(BADGE_NOT_FOUND_TOAST, 'info')");
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

  it('reads Rust directly and forced, never through the store', () => {
    expect(text).toContain('getBadgeWall(badgeSortRef.current, true)');
    expect(text).not.toContain('loadBadgeGallery(badgeSortRef.current, true)');
  });

  it('follows with a plain read at the sort current at answer time', () => {
    expect(text).toContain('void loadBadgeGallery(badgeSortRef.current, false)');
  });

  it('takes its key from the render closure, never from getState', () => {
    expect(text).not.toMatch(/getState\(\)\.badgeLinkKey/);
    expect(text).toContain('const key = badgeLinkKey;');
  });

  it('cancels on unmount in its own empty-deps effect and the link effect has no cleanup', () => {
    expect(text).toMatch(/useEffect\(\(\) => \(\) => \{\n\s+linkGen\.current \+= 1;\n\s+\}, \[\]\);/);
    const start = text.indexOf('if (!badgeLinkKey) return;');
    const end = text.indexOf('}, [badgeLinkKey, addToast]);');
    expect(end).toBeGreaterThan(start);
    expect(text.slice(start, end)).not.toMatch(/return \(\) =>/);
  });
});
