// The global Twitch badge wall, held OUTSIDE the Rewards screen.
//
// The tab shell unmounts a screen the moment you leave it, so state kept in
// RewardsScreen was gone on every visit and the wall rebuilt from zero each
// time: the cached badge set, the whole metadata cache over IPC, your earned
// set, then a metadata backfill that re-ran for anything still missing. Kept
// here, a revisit renders what was on screen a moment ago and only re-reads
// when the data is stale, was pushed to (`badge-standing-changed`), or you
// pull to refresh. The backfill runs once per session. What you own, each
// badge's earn window and what you are missing come from Rust's badge
// standing; this store only holds what the screen renders.
import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import {
  getBadgeStanding,
  windowStatusAt,
  type BadgeStanding,
  type MissingBadge,
  type WindowRun,
} from '../../services/badgeStanding';
import { formatBadgeDateInfo, type BadgeWindowStatus } from '../../utils/badgeWindow';
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
  /** The earn window Rust resolved, for reading against the clock. */
  window: WindowRun[] | null;
  /** Every `set/version` this tile stands for. Tiles are merged by title, and
   *  owning any of the merged badges means owning the tile. */
  keys: string[];
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

function build(
  global: GlobalBadgeResponse | null,
  metaMap: Record<string, CachedBadgeMeta>,
  windows: Record<string, WindowRun[]>,
): GlobalBadge[] {
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
      const ownKey = `${set.set_id}/${v.id}`;
      const window = windows[ownKey] ?? null;
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
        status: windowStatusAt(window),
        window,
        keys: [ownKey],
        dateInfo: formatBadgeDateInfo(cached?.data?.more_info),
        moreInfo: cached?.data?.more_info ?? '',
        infoUrl: cached?.data?.info_url ?? '',
      };
      const prev = byTitle.get(v.title);
      if (!prev) {
        byTitle.set(v.title, entry);
      } else if (prev.setId === entry.setId && versionRank(entry.versionId) > versionRank(prev.versionId)) {
        byTitle.set(v.title, { ...entry, keys: [...prev.keys, ownKey] });
      } else {
        prev.keys.push(ownKey);
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
  /** Owned badge ids (`set/version`), from the Rust badge standing. */
  owned: Set<string>;
  /** Unowned badges earnable right now, ending soonest first (Rust-built). */
  missingNow: MissingBadge[];
  standing: BadgeStanding | null;
  loading: boolean;
  /** Badges whose metadata is still being fetched; 0 when idle. */
  metaProgress: number;
  loadedAt: number;
  /** The one-per-session metadata backfill has run. */
  backfillDone: boolean;
}

export const useBadgeGallery = create<BadgeGalleryState>(() => ({
  badges: [],
  owned: new Set(),
  missingNow: [],
  standing: null,
  loading: false,
  metaProgress: 0,
  loadedAt: 0,
  backfillDone: false,
}));

// The last catalogue and metadata the wall was built from, so a standing that
// arrives on its own (a refresh landing, a window boundary) can re-stamp the
// tiles without re-reading either.
let lastGlobal: GlobalBadgeResponse | null = null;
let lastMeta: Record<string, CachedBadgeMeta> = {};

/** Take a standing from Rust: ownership, the missing list, and the windows the tiles read. */
export function applyStanding(standing: BadgeStanding): void {
  useBadgeGallery.setState({
    standing,
    owned: new Set(standing.owned),
    missingNow: standing.missing_now,
    ...(lastGlobal ? { badges: build(lastGlobal, lastMeta, standing.windows) } : {}),
  });
}

/** Re-ask Rust for the standing (a refresh landed, or a window opened or closed). */
export function refreshBadgeStanding(force = false): Promise<void> {
  return getBadgeStanding(force)
    .then(applyStanding)
    .catch((err) => Logger.warn('[Rewards] badge standing unavailable:', err));
}

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
export function loadBadgeGallery(opts: { force?: boolean } = {}): Promise<void> {
  if (!opts.force && badgeGalleryIsWarm()) return Promise.resolve();
  if (inFlight) return inFlight;
  inFlight = run(opts).finally(() => {
    inFlight = null;
  });
  return inFlight;
}

async function run(opts: { force?: boolean }): Promise<void> {
  const set = useBadgeGallery.setState;
  set({ loading: true });
  try {
    // Ownership, windows and the missing list come from Rust, which answers
    // from its cache at once; asked alongside the catalogue read.
    const standingRequest = getBadgeStanding(opts.force ?? false).catch((err) => {
      Logger.warn('[Rewards] badge standing unavailable:', err);
      return null;
    });

    let global = await invoke<GlobalBadgeResponse | null>('get_cached_global_badges');
    if (!global?.data?.length) {
      await invoke('prefetch_global_badges').catch(() => {});
      global = await invoke<GlobalBadgeResponse | null>('get_cached_global_badges');
    }

    // Badge metadata (newest-first position, dates, usage) comes from the
    // universal cache in one batch, keyed exactly as the desktop gallery keys it.
    lastGlobal = global;
    lastMeta = await readMeta();
    const standing = await standingRequest;
    if (standing) {
      applyStanding(standing);
    } else {
      set({ badges: build(global, lastMeta, {}) });
    }
    set({ loading: false, loadedAt: Date.now() });

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
        lastMeta = await readMeta();
        // The backfill can carry earn windows the standing has not seen yet;
        // taking the new standing rebuilds the tiles from the refreshed cache.
        const refreshed = await getBadgeStanding(false).catch(() => null);
        if (refreshed) {
          applyStanding(refreshed);
        } else {
          set({ badges: build(global, lastMeta, useBadgeGallery.getState().standing?.windows ?? {}) });
        }
        set({ loadedAt: Date.now() });
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
