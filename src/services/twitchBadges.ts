import { Logger } from '../utils/logger';
/**
 * DEPRECATED: This service has been replaced by the unified Rust badge service
 * 
 * All badge functionality is now handled in Rust for maximum performance.
 * Please use `badgeService.ts` instead:
 * 
 * ```typescript
 * import { getAllUserBadges, prefetchChannelBadges } from './badgeService';
 * ```
 * 
 * The Rust backend handles:
 * - Twitch Helix API (global + channel badges)
 * - Twitch GQL (user display/earned badges)
 * - Third-party providers (FFZ, Chatterino, Homies)
 * - LRU caching with automatic eviction
 * - Background pre-fetching
 */

// Re-export from unified service for backwards compatibility
export {
  getAllUserBadges,
  parseBadgeString,
  prefetchGlobalBadges,
  prefetchChannelBadges,
  clearBadgeCache,
  clearChannelBadgeCache,
  type BadgeInfo,
  type UserBadge,
  type UserBadgesResponse,
  type TwitchBadge,
} from './badgeService';

// Legacy function names for backwards compatibility
export { prefetchGlobalBadges as fetchGlobalBadges } from './badgeService';

// Older callsites expect a 3-arg `fetchChannelBadges(channelId, clientId, token)`.
// Keep that signature but ignore extra args.
export async function fetchChannelBadges(channelId: string, _clientId?: string, _token?: string): Promise<void> {
  const { prefetchChannelBadges } = await import('./badgeService');
  await prefetchChannelBadges(channelId);
}

/**
 * @deprecated Badge info is now included in the UserBadgesResponse from getAllUserBadges()
 * This stub returns null to maintain backwards compatibility
 */

// Prebuilt lookup indexes. Global badges come from `commands/badges.rs`
// (universal cache); channel badges are per-room-id and must be fetched
// separately - many chat-visible badges (subscriber, bits, etc.) are
// channel-scoped. getBadgeInfoFromCache runs per badge of every parsed
// message (and parseMessage runs per row), so the nested set/version array
// scans were O(sets x versions) on the chat hot path; these make it two Map
// hits with the info objects constructed once at index time.
type BadgeVersionInfo = Record<string, unknown>;
type BadgeIndex = Map<string, Map<string, BadgeVersionInfo>>;
let globalBadgeIndex: BadgeIndex | null = null;
const channelBadgeIndexes = new Map<string, BadgeIndex>();

function buildBadgeIndex(payload: any): BadgeIndex {
  const index: BadgeIndex = new Map();
  if (!payload?.data) return index;
  for (const badgeSet of payload.data) {
    const versions = new Map<string, BadgeVersionInfo>();
    for (const v of badgeSet.versions ?? []) {
      versions.set(v.id, {
        image_url_1x: v.image_url_1x,
        image_url_2x: v.image_url_2x,
        image_url_4x: v.image_url_4x,
        title: v.title,
        description: v.description,
        click_action: v.click_action,
        click_url: v.click_url,
      });
    }
    index.set(badgeSet.set_id, versions);
  }
  return index;
}

/**
 * Initialize badge cache from Rust.
 *
 * This must complete BEFORE we start consuming chat messages, otherwise
 * `parseBadges()` will return `{info:null}` and ChatMessage won't render them.
 */
export async function initializeBadgeCache(channelId?: string): Promise<void> {
  try {
    const { invoke } = await import('@tauri-apps/api/core');

    // ------------------------------------------------------------
    // Global badges (cached on disk via universal cache)
    // ------------------------------------------------------------
    let globalBadges = await invoke('get_cached_global_badges');

    if (!globalBadges) {
      // Not cached, fetch+cache via the non-unified badge command.
      // (The unified badge service caches in memory only, and won't populate
      // `get_cached_global_badges`.)
      Logger.debug('[BadgeCache] Global badges not cached, prefetching...');
      await invoke('prefetch_global_badges');
      globalBadges = await invoke('get_cached_global_badges');
    }

    if (globalBadges) {
      globalBadgeIndex = buildBadgeIndex(globalBadges);
      Logger.debug('[BadgeCache] Loaded global badges into memory cache');
    } else {
      Logger.warn('[BadgeCache] Failed to load global badges even after prefetch');
    }

    // ------------------------------------------------------------
    // Channel badges (not stored in universal cache in this codepath)
    // ------------------------------------------------------------
    if (channelId) {
      try {
        // Rust attaches the credentials itself.
        const channelBadges = await invoke<any>('fetch_channel_badges', { channelId });

        channelBadgeIndexes.set(channelId, buildBadgeIndex(channelBadges));
        Logger.debug('[BadgeCache] Loaded channel badges into memory cache for:', channelId);
      } catch (e) {
        Logger.warn('[BadgeCache] Failed to fetch channel badges:', e);
      }

      // Still prefetch in unified service so other codepaths (profile lookups)
      // can take advantage of the warmed in-memory cache.
      try {
        await invoke('prefetch_channel_badges_unified', { channelId });
      } catch {
        // ignore
      }
    }
  } catch (error) {
    Logger.warn('[BadgeCache] Failed to initialize badge cache:', error);
  }
}

/**
 * Legacy function for parsing badge strings
 * Enriches badges with metadata from in-memory cache
 */
export function parseBadges(badgeString: string, channelId?: string): Array<{ key: string; info: any }> {
  if (!badgeString) return [];

  return badgeString.split(',').map((badge) => {
    const [name, version] = badge.split('/');
    const key = `${name}/${version}`;

    // Look up badge info from in-memory cache (channel first, then global)
    const info = getBadgeInfoFromCache(name, version, channelId);

    return {
      key,
      info,
    };
  });
}

/**
 * Get badge info from in-memory cache (synchronous). Channel badges win
 * (subscriber, bits, etc.), then the global set.
 */
function getBadgeInfoFromCache(setId: string, versionId: string, channelId?: string): any | null {
  if (channelId) {
    const hit = channelBadgeIndexes.get(channelId)?.get(setId)?.get(versionId);
    if (hit) return hit;
  }
  return globalBadgeIndex?.get(setId)?.get(versionId) ?? null;
}
