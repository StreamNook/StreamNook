// The gallery's Newest and Oldest orders and the detail line's date are Rust's
// (services/badge_index.rs). This pins the lookups that read them and the
// source of the three surfaces that must never parse a date of their own again
// (a node test cannot render them, so the calls are pinned in their source).
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { AddedDate, BadgeStanding } from '../../services/badgeStanding';
import { addedLabelFor, compareNewest, latestOnly, seedStanding } from './badgeAddedOrder';

const badge = (set_id: string, id: string) => ({ set_id, id });

function added(places: Record<string, number>): Record<string, AddedDate> {
  return Object.fromEntries(
    Object.entries(places).map(([key, order]) => [key, { ms: null, precision: null, label: null, order }]),
  );
}

const names = (list: { set_id: string; id: string }[]) => list.map((b) => `${b.set_id}/${b.id}`);

const read = (relative: string) =>
  readFileSync(new URL(relative, import.meta.url), 'utf8').replace(/\r\n/g, '\n');

describe('compareNewest', () => {
  const list = [badge('c', '1'), badge('a', '1'), badge('b', '1')];

  it('orders by the place Rust gave each badge', () => {
    const map = added({ 'a/1': 0, 'b/1': 1, 'c/1': 2 });
    expect(names([...list].sort((x, y) => compareNewest(map, x, y)))).toEqual(['a/1', 'b/1', 'c/1']);
  });

  it('puts a badge with no place first and settles the rest by place', () => {
    const map = added({ 'a/1': 3, 'b/1': 1 });
    // c has no place: first, as an undated badge is.
    expect(names([...list].sort((x, y) => compareNewest(map, x, y)))).toEqual(['c/1', 'b/1', 'a/1']);
  });

  it('settles by the lowercased key by code unit while the standing is null', () => {
    const pair = [badge('a_b', '1'), badge('a-b', '1'), badge('H1Z1_1', '1')];
    // "-" (45) sorts before "_" (95) by code unit; a locale compare orders them the other way.
    expect(names([...pair].sort((x, y) => compareNewest(null, x, y)))).toEqual(['a-b/1', 'a_b/1', 'H1Z1_1/1']);
    expect(names([...pair].sort((x, y) => compareNewest(undefined, x, y)))).toEqual([
      'a-b/1',
      'a_b/1',
      'H1Z1_1/1',
    ]);
  });

  it('never returns NaN and is symmetric', () => {
    const map = added({ 'a/1': 0 });
    for (const x of list) {
      for (const y of list) {
        const forward = compareNewest(map, x, y);
        expect(Number.isNaN(forward)).toBe(false);
        expect(Math.sign(forward)).toBe(-Math.sign(compareNewest(map, y, x)) || 0);
      }
    }
  });

  it('reads Oldest First as the exact reverse', () => {
    const map = added({ 'a/1': 0, 'b/1': 1, 'c/1': 2 });
    expect(names([...list].sort((x, y) => compareNewest(map, y, x)))).toEqual(['c/1', 'b/1', 'a/1']);
  });
});

describe('addedLabelFor', () => {
  it('reads only the standing, never a metadata date', () => {
    const map: Record<string, AddedDate> = {
      'chain/1': { ms: 1, precision: 'day', label: '13 November 2026', order: 0 },
    };
    expect(addedLabelFor(map, 'chain', '1')).toBe('13 November 2026');
    // A badge the standing does not date shows nothing, whatever a scraped date says elsewhere.
    expect(addedLabelFor(map, 'other', '1')).toBeNull();
    expect(addedLabelFor(null, 'chain', '1')).toBeNull();
    expect(addedLabelFor(undefined, 'chain', '1')).toBeNull();
  });
});

describe('seedStanding', () => {
  const standing = (login: string | null) => ({ login }) as BadgeStanding;

  it('uses the last standing for the same login, whatever its case', () => {
    const last = standing('Tyler');
    expect(seedStanding(last, 'tyler')).toBe(last);
  });

  it('never uses a standing that is for another login', () => {
    expect(seedStanding(standing('tyler'), 'someone_else')).toBeNull();
    // Signed in now, last standing was signed out, and the reverse.
    expect(seedStanding(standing(null), 'tyler')).toBeNull();
    expect(seedStanding(standing('tyler'), null)).toBeNull();
    expect(seedStanding(standing('tyler'), undefined)).toBeNull();
  });

  it('seeds a signed-out gallery from a signed-out standing, and from nothing it seeds nothing', () => {
    const last = standing(null);
    expect(seedStanding(last, undefined)).toBe(last);
    expect(seedStanding(null, 'tyler')).toBeNull();
  });
});

describe('latestOnly', () => {
  it('drops an older answer that resolves after a newer one', async () => {
    const seen: string[] = [];
    const apply = latestOnly<string>((value) => {
      seen.push(value);
    });
    let resolveOld: (value: string) => void = () => {};
    const old = apply(
      new Promise<string>((resolve) => {
        resolveOld = resolve;
      }),
    );
    const fresh = apply(Promise.resolve('newer'));
    await fresh;
    resolveOld('older');
    await old;
    expect(seen).toEqual(['newer']);
  });

  it('applies answers that arrive in order', async () => {
    const seen: string[] = [];
    const apply = latestOnly<string>((value) => {
      seen.push(value);
    });
    await apply(Promise.resolve('first'));
    await apply(Promise.resolve('second'));
    expect(seen).toEqual(['first', 'second']);
  });

  it('lets a failed request reject its own promise and apply nothing', async () => {
    const apply = latestOnly<string>(() => {
      throw new Error('must not apply');
    });
    await expect(apply(Promise.reject(new Error('down')))).rejects.toThrow('down');
  });
});

describe('the surfaces read the date Rust resolved', () => {
  it('the gallery sorts by the standing and keeps no date parser or rank branch', () => {
    const overlay = read('../BadgesOverlay.tsx');
    expect(overlay).not.toContain('parseDate');
    expect(overlay).not.toContain('canUsePositions');
    expect(overlay).not.toContain('addedAt(');
    // The order is the same for every account, so it reads the account-free copy.
    expect(overlay).toContain('compareNewest(orderStanding?.added');
    expect(overlay).toContain('windowStatusAt(orderStanding?.windows[');
    expect(overlay).toContain('latestOnly<BadgeStanding>(');
    // Reopened, it starts from the last standing; with none yet, the date sorts wait
    // on the grid's spinner unless the read failed.
    expect(overlay).toContain('seedStanding(getLastStanding(), me?.login ?? me?.username)');
    // Only the order copy starts from any account's last standing; what the account
    // owns never does.
    expect(overlay.split('(() => getLastStanding())').length - 1).toBe(1);
    expect(overlay).toContain(
      'const [orderStanding, setOrderStanding] = useState<BadgeStanding | null>(() => getLastStanding());',
    );
    expect(overlay).toContain('orderStanding === null &&');
    expect(overlay).toContain('setStandingFailed(true)');
    // An account switch with the gallery open drops the old account's standing and reads again.
    expect(overlay).toContain(
      'const accountKey = (currentUser?.login ?? currentUser?.username ?? null)?.toLowerCase() ?? null;',
    );
    expect(overlay).toContain('setStanding((s) => seedStanding(s, accountKey));');
    expect(overlay).toContain('}, [accountKey]);');
    expect(overlay).toContain('(loading || waitingForDates) && !error && (');
    expect(overlay).toContain('!loading && !waitingForDates && !error');
    expect(overlay).toContain("sortBy === 'date-newest' || sortBy === 'date-oldest' || sortBy === 'available' || sortBy === 'coming-soon'");
    expect(read('../../services/badgeStanding.ts')).toContain('lastStanding = standing;');
  });

  it('the detail overlay shows only the standing label, asks for it on open and re-reads it', () => {
    const detail = read('../BadgeDetailOverlay.tsx');
    // A scraped metadata date, present or not, is never shown.
    expect(detail).not.toContain('badgeBaseInfo?.date_added');
    expect(detail).not.toContain('badgeBaseInfo.date_added');
    expect(detail).toContain('getBadgeStanding(false)');
    expect(detail).toContain('addedLabelFor(added, setId, badge.id)');
    expect(detail).toContain("listen('badge-standing-changed', read)");
    expect(detail).toContain('badgeBaseInfo?.usage_stats || addedLabel || isDrops');
    expect(detail).toContain('Added {addedLabel}');
  });

  it('the phone sheet shows the tile label Rust worked out', () => {
    const rewards = read('../../mobile/screens/RewardsScreen.tsx');
    expect(rewards).toContain('Added {badgeDetail.added_label}');
  });
});
