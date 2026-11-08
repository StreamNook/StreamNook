// The webview twin of Rust's `services/badge_fold.rs`. It lives in TS because a
// desktop row's final badge set is assembled here from live stores (7TV and
// add-on badges arrive late through chatUserStore); one IPC per row on the
// chat hot path would cost far more than this rule. Rust and this file read
// the same rules (badgeFold.rules.json) and run the same cases.
import rules from './badgeFold.rules.json';

export type FoldKind = 'role' | 'platform' | 'seventv' | 'addon' | 'streamnook';

export interface BadgeFold {
  shown: number[];
  hidden: number[];
  /** How rare the chatter's add-on collection is: 0 plain, 1 uncommon, 2 rare, 3 epic, 4 legendary. */
  tier: number;
}

export const DEFAULT_BADGE_FOLD_LIMIT = 0;

const ROLE_SETS = new Set(rules.role_sets);
const PLATFORM_PROVIDERS = new Set(rules.platform_providers);

/** A badge's kind from its provider key and (for platform badges) its set name. */
export function foldKind(provider: string, setName: string): FoldKind {
  const p = provider.toLowerCase();
  if (p === '7tv') return 'seventv';
  if (p === 'streamnook') return 'streamnook';
  if (PLATFORM_PROVIDERS.has(p)) return ROLE_SETS.has(setName) ? 'role' : 'platform';
  return 'addon';
}

/** The setting as a usable limit of add-on badges: 0 = show all, else 2 to 5; missing = show all. */
export function badgeFoldLimit(setting: number | undefined): number {
  if (setting === undefined || !Number.isFinite(setting)) return DEFAULT_BADGE_FOLD_LIMIT;
  const n = Math.floor(setting);
  if (n <= 0) return 0;
  return Math.min(5, Math.max(2, n));
}

/** An add-on badge: the only kind the limit folds and the rarity tier counts. */
const isAddon = (k: FoldKind) => k === 'seventv' || k === 'addon' || k === 'streamnook';

/** The rarity tier for a count of add-on badges; the viewer's limit never changes it. */
export function rarityTier(addons: number): number {
  return rules.tiers.filter((start) => addons >= start).length;
}

/**
 * Folds a row's badges for `limit` add-ons (0 = show all). The platform's own
 * badges always show and never count. Add-ons keep the StreamNook badge first,
 * then the rest in chat order; the rest fold into the chip, even a single one.
 */
export function foldBadges(kinds: readonly FoldKind[], limit: number): BadgeFold {
  const addons = kinds.flatMap((k, i) => (isAddon(k) ? [i] : []));
  const tier = rarityTier(addons.length);
  if (limit <= 0 || addons.length <= limit) return { shown: kinds.map((_, i) => i), hidden: [], tier };
  const kept = [
    ...addons.filter((i) => kinds[i] === 'streamnook'),
    ...addons.filter((i) => kinds[i] !== 'streamnook'),
  ].slice(0, limit);
  return {
    shown: kinds.flatMap((k, i) => (!isAddon(k) || kept.includes(i) ? [i] : [])),
    hidden: addons.filter((i) => !kept.includes(i)),
    tier,
  };
}
