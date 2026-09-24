// The signed-in account's badge standing, built in Rust
// (services/badge_standing.rs): what it owns, when every badge can be earned,
// and what it is missing that is earnable right now. This module only fetches
// it and turns it into words; the collection read, the window parsing and the
// missing list all live in Rust.
import { invoke } from '@tauri-apps/api/core';
import type { BadgeWindowStatus } from '../utils/badgeWindow';

/** One span a badge is earnable in. `null` is an open end. */
export interface WindowRun {
  start_ms: number | null;
  end_ms: number | null;
}

export type CollectionState = 'complete' | 'partial' | 'unavailable';

/** One way a badge is earned, as Rust's badge_earn.rs classifies it. */
export type EarnStep =
  | { kind: 'purchase'; ticket: boolean }
  | { kind: 'subscribe'; count: number | null }
  | { kind: 'watch'; minutes: number | null; days: number | null }
  | { kind: 'cheer' }
  | { kind: 'create' }
  | { kind: 'attend' }
  | { kind: 'other' };

export interface EarnPath {
  /** In the order they happen: pay or sub first, then watch. */
  steps: EarnStep[];
  /** The reward is drawn at random from this many badges. */
  random_of: number | null;
  /** Read out of the badge's description rather than the campaign's numbers. */
  inferred: boolean;
  /** The sentence it came from, for a tooltip. */
  detail: string | null;
}

export interface MissingBadge {
  /** `set_id/version`. */
  key: string;
  set_id: string;
  version: string;
  title: string;
  image_url: string;
  ends_ms: number | null;
  earn: EarnPath;
  category: string | null;
}

export interface BadgeStanding {
  login: string | null;
  collection: CollectionState;
  collection_reason: 'not_signed_in' | 'fetch_failed' | null;
  stale: boolean;
  refreshing: boolean;
  catalogue_ready: boolean;
  owned: string[];
  windows: Record<string, WindowRun[]>;
  missing_now: MissingBadge[];
  next_change_ms: number | null;
  /** Twitch's own art for earn chips, by step kind (`subscribe`: the gift, `cheer`: the gem). */
  earn_icons: Record<string, string>;
  /** What earning everything in `missing_now` takes; null when it is empty. */
  catch_up: CatchUp | null;
  generated_ms: number;
}

/** Totals for earning every missing badge, effort shared within a category (Rust `CatchUp`). */
export interface CatchUp {
  subs: number;
  /** `subs` at the US Tier 1 price. */
  sub_cost_cents: number;
  watch_minutes: number;
  /** Event passes; priced by the event, so not in the cost. */
  tickets: number;
  /** Badges drawn at random: the totals are a floor for them. */
  random: number;
  /** Badges with no number to add up (cheer, create, other). */
  unpriced: number;
  estimated: boolean;
}

/** Rust answers from its cache at once; a due refresh follows as `badge-standing-changed`. */
export function getBadgeStanding(force = false): Promise<BadgeStanding> {
  return invoke<BadgeStanding>('get_badge_standing', { force });
}

export function getBadgeWindow(setId: string, version: string): Promise<WindowRun[] | null> {
  return invoke<WindowRun[] | null>('get_badge_window', { setId, version });
}

/**
 * Where the clock sits against a Rust-resolved window. Kept here rather than
 * asked of Rust: it is two comparisons against the wall clock at render time,
 * and asking over IPC every time a tile renders would cost more than the
 * comparison. The parsing that produced the runs is the part Rust owns.
 */
export function windowStatusAt(
  runs: WindowRun[] | null | undefined,
  now: number = Date.now(),
): BadgeWindowStatus | null {
  if (!runs?.length) return null;
  const start = (r: WindowRun) => r.start_ms ?? -Infinity;
  const end = (r: WindowRun) => r.end_ms ?? Infinity;
  if (runs.some((r) => now >= start(r) && now <= end(r))) return 'available';
  if (runs.some((r) => now < start(r))) return 'coming-soon';
  return 'expired';
}

const MAX_TIMER_MS = 6 * 60 * 60 * 1000;

/**
 * Delay until the next window boundary, for a single re-ask. Clamped: a
 * setTimeout delay past 2^31-1 ms fires immediately, which for a boundary a
 * month out would re-ask in a tight loop. `null` = nothing to schedule.
 */
export function refetchDelay(nextChangeMs: number | null | undefined, now: number = Date.now()): number | null {
  if (nextChangeMs == null) return null;
  return Math.min(Math.max(nextChangeMs - now + 500, 1000), MAX_TIMER_MS);
}

/** The short text on an earn chip: "30 min", "20 min × 3d", "2 subs", "Ticket". */
export function earnChipText(step: EarnStep): string {
  switch (step.kind) {
    case 'watch': {
      const base = step.minutes ? formatMinutes(step.minutes) : 'Watch';
      return step.days ? `${base} × ${step.days}d` : base;
    }
    case 'subscribe':
      return step.count ? `${step.count} subs` : 'Sub';
    case 'purchase':
      return step.ticket ? 'Ticket' : 'Paid';
    case 'cheer':
      return 'Bits';
    case 'create':
      return 'Creator';
    case 'attend':
      return 'In person';
    case 'other':
      return 'Special';
  }
}

/** The longer form, for the chip's tooltip. */
export function earnChipHint(step: EarnStep): string {
  switch (step.kind) {
    case 'watch':
      return step.minutes
        ? `Watch ${formatMinutes(step.minutes)}${step.days ? ` on ${step.days} different days` : ''}`
        : 'Watch during the campaign';
    case 'subscribe':
      return step.count ? `Subscribe or gift ${step.count} subscriptions` : 'Subscribe or gift a subscription';
    case 'purchase':
      return step.ticket ? 'Granted with a ticket purchase' : 'Granted with a purchase';
    case 'cheer':
      return 'Cheer with Bits';
    case 'create':
      return 'Something you do as a creator';
    case 'attend':
      return 'Be there in person';
    case 'other':
      return 'See the details';
  }
}

function formatMinutes(minutes: number): string {
  if (minutes < 60 || minutes % 60 !== 0) return `${minutes} min`;
  return `${minutes / 60} hr`;
}

/** "3d left", "5h left", "ends soon"; empty when the window has no end. */
export function timeLeftLabel(endsMs: number | null, now: number = Date.now()): string {
  if (endsMs == null) return '';
  const left = endsMs - now;
  if (left <= 0) return 'ending';
  const hours = Math.floor(left / 3_600_000);
  if (hours >= 48) return `${Math.floor(hours / 24)}d left`;
  if (hours >= 1) return `${hours}h left`;
  return 'ends soon';
}

/** "$17.97" from cents. */
export function usdLabel(cents: number): string {
  return new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD' }).format(cents / 100);
}

/** "45m", "6h 20m", "2d 3h": watch time at a glance. */
export function watchTimeLabel(minutes: number): string {
  const m = Math.max(0, Math.round(minutes));
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 48) return m % 60 ? `${h}h ${m % 60}m` : `${h}h`;
  return h % 24 ? `${Math.floor(h / 24)}d ${h % 24}h` : `${Math.floor(h / 24)}d`;
}

/** The same countdown as a phrase to finish a sentence: "in 6d", "in 5h", "soon". */
export function closesInLabel(endsMs: number | null, now: number = Date.now()): string {
  if (endsMs == null) return '';
  const hours = Math.floor((endsMs - now) / 3_600_000);
  if (hours >= 48) return `in ${Math.floor(hours / 24)}d`;
  if (hours >= 1) return `in ${hours}h`;
  return 'soon';
}
