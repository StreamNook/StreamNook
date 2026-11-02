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
}

export const DEFAULT_BADGE_FOLD_LIMIT = 3;

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

/** The setting as a usable limit: 0 = show all, else 2 to 5; missing = 3. */
export function badgeFoldLimit(setting: number | undefined): number {
  if (setting === undefined || !Number.isFinite(setting)) return DEFAULT_BADGE_FOLD_LIMIT;
  const n = Math.floor(setting);
  if (n <= 0) return 0;
  return Math.min(5, Math.max(2, n));
}

export function foldBadges(kinds: readonly FoldKind[], limit: number): BadgeFold {
  if (limit <= 0 || kinds.length <= limit + 1) return { shown: kinds.map((_, i) => i), hidden: [] };
  const first = (k: FoldKind) => kinds.indexOf(k);
  const platform = first('role') >= 0 ? first('role') : first('platform');
  const picks: number[] = [];
  for (const i of [platform, first('seventv'), first('streamnook')]) {
    if (i >= 0 && picks.length < limit) picks.push(i);
  }
  for (let i = 0; i < kinds.length && picks.length < limit; i++) {
    if (!picks.includes(i)) picks.push(i);
  }
  picks.sort((a, b) => a - b);
  return { shown: picks, hidden: kinds.map((_, i) => i).filter((i) => !picks.includes(i)) };
}
