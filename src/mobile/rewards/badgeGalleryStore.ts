// The global Twitch badge wall, held OUTSIDE the Rewards screen.
//
// The tab shell unmounts a screen the moment you leave it, so state kept in
// RewardsScreen was gone on every visit and the wall rebuilt from zero each
// time: the cached badge set, the whole metadata cache over IPC, your earned
// set, then a metadata backfill that re-ran for anything still missing. Kept
// here, a revisit renders what was on screen a moment ago and only re-reads
// when the data is stale, was pushed to (`badge-metadata-amended`), or you
// pull to refresh. The backfill runs once per session.
import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { getAllUserBadgesWithEarned } from '../../services/badgeService';
import { deriveBadgeStatus, formatBadgeDateInfo, type BadgeWindowStatus } from '../../utils/badgeWindow';
import { Logger } from '../../utils/logger';

export interface GlobalBadge {
  key: string;
  setId: string;
  versionId: string;
  title: string;
  description: string;
  image: string;
  /** Precomputed newest-first rank from the badge metadata cache. */
  position: number;
  /** Unix ms the badge was added, 0 when unknown. */
  addedMs: number;
  usage: number;
  status: BadgeWindowStatus | null;
  dateInfo: string;
  moreInfo: string;
  infoUrl: string;
}

interface GlobalBadgeVersion {
  id?: string;
  title?: string;
  description?: string;
  image_url_2x?: string;
  image_url_4x?: string;
}
interface GlobalBadgeSet {
  set_id?: string;
  versions?: GlobalBadgeVersion[];
}
interface GlobalBadgeResponse {
  data?: GlobalBadgeSet[];
}
interface CachedBadgeMeta {
  data?: {
    date_added?: string | null;
    usage_stats?: string | null;
    more_info?: string | null;
    enrichment?: Record<string, unknown> | null;
    info_url?: string;
  };
  position?: number;
}

/** A wall read within this window is served from memory. */
const FRESH_FOR_MS = 15 * 60 * 1000;

// "1,234 users" -> 1234, so the usage sort has something numeric to work with.
function parseUsage(raw: string | null | undefined): number {
  if (!raw) return 0;
  const digits = raw.replace(/[^0-9]/g, '');
  return digits ? parseInt(digits, 10) : 0;
}

function parseAdded(raw: string | null | undefined): number {
  if (!raw) return 0;
  const ms = new Date(raw).getTime();
  return Number.isNaN(ms) ? 0 : ms;
}

// A pushed badge has no scraped date_added yet, but its relay enrichment
// carries the campaign window; the window opening is an honest "how new is
// this" stand-in, and without it a fresh badge sorts as if it were ancient.
function enrichmentStartMs(meta: CachedBadgeMeta | undefined): number {
  const raw = meta?.data?.enrichment?.['starts_utc'];
  if (typeof raw !== 'string') return 0;
  const ms = new Date(raw).getTime();
  return Number.isNaN(ms) ? 0 : ms;
}

// Version ids are numeric strings in practice; compare them as numbers so
// "10" beats "9", falling back to string order for anything exotic.
function versionRank(id: string): number {
  const n = parseInt(id, 10);
  return Number.isNaN(n) ? 0 : n;
}

function build(global: GlobalBadgeResponse | null, metaMap: Record<string, CachedBadgeMeta>): GlobalBadge[] {
  // Keyed by title. The same badge genuinely repeats across sets (keep the
  // first), but a REVISION arrives as a higher version id in the SAME set with
  // the same title, and it must replace the original: first-wins here is how
  // the gallery kept rendering a retired revision's window ("Ended") for a
  // badge that had just relaunched, and why pull-to-refresh appeared to do
  // nothing.
  const byTitle = new Map<string, GlobalBadge>();
  for (const set of global?.data ?? []) {
    for (const v of set.versions ?? []) {
      const image = v.image_url_4x || v.image_url_2x;
      if (!v.title || !image || !set.set_id || !v.id) continue;
      const cached = metaMap[`metadata:${set.set_id}-v${v.id}`];
      const entry: GlobalBadge = {
        key: `${set.set_id}-${v.id}`,
        setId: set.set_id,
        versionId: v.id,
        title: v.title,
        description: v.description ?? '',
        image,
        position: typeof cached?.position === 'number' ? cached.position : Number.MAX_SAFE_INTEGER,
        addedMs: parseAdded(cached?.data?.date_added) || enrichmentStartMs(cached),
        usage: parseUsage(cached?.data?.usage_stats),
        status: deriveBadgeStatus(cached?.data?.more_info, cached?.data?.enrichment),
        dateInfo: formatBadgeDateInfo(cached?.data?.more_info),
        moreInfo: cached?.data?.more_info ?? '',
        infoUrl: cached?.data?.info_url ?? '',
      };
      const prev = byTitle.get(v.title);
      if (!prev || (prev.setId === entry.setId && versionRank(entry.versionId) > versionRank(prev.versionId))) {
        byTitle.set(v.title, entry);
      }
    }
  }
  return [...byTitle.values()];
}

async function readMeta(): Promise<Record<string, CachedBadgeMeta>> {
  try {
    return (await invoke<Record<string, CachedBadgeMeta>>('get_all_universal_cached_items', { cacheType: 'badge' })) ?? {};
  } catch (err) {
    Logger.warn('[Rewards] badge metadata cache unavailable:', err);
    return {};
  }
}

interface BadgeGalleryState {
  badges: GlobalBadge[];
  ownedTitles: Set<string>;
  loading: boolean;
  /** Badges whose metadata is still being fetched; 0 when idle. */
  metaProgress: number;
  loadedAt: number;
  /** The one-per-session metadata backfill has run. */
  backfillDone: boolean;
}

export const useBadgeGallery = create<BadgeGalleryState>(() => ({
  badges: [],
  ownedTitles: new Set(),
  loading: false,
  metaProgress: 0,
  loadedAt: 0,
  backfillDone: false,
}));

/** True when the wall can render straight from memory. */
export function badgeGalleryIsWarm(): boolean {
  const s = useBadgeGallery.getState();
  return s.badges.length > 0 && Date.now() - s.loadedAt < FRESH_FOR_MS;
}

let inFlight: Promise<void> | null = null;

/**
 * Populate the wall. Served from memory while fresh unless `force`; one call
 * at a time, later callers joining the one in flight.
 */
export function loadBadgeGallery(opts: { userId?: string; login?: string; force?: boolean }): Promise<void> {
  if (!opts.force && badgeGalleryIsWarm()) return Promise.resolve();
  if (inFlight) return inFlight;
  inFlight = run(opts).finally(() => {
    inFlight = null;
  });
  return inFlight;
}

async function run(opts: { userId?: string; login?: string }): Promise<void> {
  const set = useBadgeGallery.setState;
  set({ loading: true });
  try {
    let global = await invoke<GlobalBadgeResponse | null>('get_cached_global_badges');
    if (!global?.data?.length) {
      await invoke('prefetch_global_badges').catch(() => {});
      global = await invoke<GlobalBadgeResponse | null>('get_cached_global_badges');
    }

    // Badge metadata (earn window + newest-first position) comes from the
    // universal cache in one batch, keyed exactly as the desktop gallery keys it.
    set({ badges: build(global, await readMeta()), loadedAt: Date.now() });

    if (opts.userId && opts.login) {
      const mine = await getAllUserBadgesWithEarned(opts.userId, opts.login, opts.userId, opts.login);
      set({ ownedTitles: new Set((mine.earnedBadges ?? []).map((b) => b.title)) });
    }
    set({ loading: false });

    // Mobile had never populated the badge metadata cache, which is why the
    // gallery had almost no dates or earn windows to sort by. Fetch what is
    // missing in batches (same commands the desktop gallery uses), then rebuild
    // from the refreshed cache. Once per session: what is still missing after
    // one pass is missing upstream, and asking again on every visit was a good
    // part of why the tab felt like it reloaded everything each time.
    if (useBadgeGallery.getState().backfillDone) return;
    set({ backfillDone: true });
    try {
      const missing = await invoke<[string, string][]>('get_badges_missing_metadata');
      if (missing.length > 0) {
        set({ metaProgress: missing.length });
        const batchSize = 5;
        for (let i = 0; i < missing.length; i += batchSize) {
          await Promise.allSettled(
            missing
              .slice(i, i + batchSize)
              .map(([setId, version]) => invoke('fetch_badge_metadata', { badgeSetId: setId, badgeVersion: version })),
          );
          set({ metaProgress: Math.max(0, missing.length - (i + batchSize)) });
        }
        set({ badges: build(global, await readMeta()), loadedAt: Date.now() });
      }
    } catch (err) {
      Logger.warn('[Rewards] badge metadata backfill failed:', err);
    } finally {
      set({ metaProgress: 0 });
    }
  } catch (err) {
    Logger.warn('[Rewards] badge load failed:', err);
  } finally {
    set({ loading: false });
  }
}
