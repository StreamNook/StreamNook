import { invoke } from '@tauri-apps/api/core';

import { Logger } from '../utils/logger';
/**
 * Unified Badge Service - Lightweight wrapper for Rust backend
 * All heavy lifting (API calls, caching, parsing) is done in Rust
 * This service transforms Rust responses to match the expected frontend format
 */

// Frontend-expected format (camelCase, flat structure)
export interface TwitchBadge {
  id: string;
  setID: string;
  version: string;
  title: string;
  description: string;
  image1x: string;
  image2x: string;
  image4x: string;
  clickAction?: string;
  clickUrl?: string;
  localUrl?: string;
}

export interface ThirdPartyBadge {
  id: string;
  title: string;
  imageUrl: string;
  image1x: string;
  image2x: string;
  image4x: string;
  provider: string;
  link?: string;
}

export interface UserBadgesResponse {
  displayBadges: TwitchBadge[];
  earnedBadges: TwitchBadge[];
  thirdPartyBadges: ThirdPartyBadge[];
  ivrBadges: any[];
}

// Rust backend response format (snake_case, nested structure)
interface RustBadgeInfo {
  id: string;
  set_id: string;
  version: string;
  title: string;
  description: string;
  image_1x: string;
  image_2x: string;
  image_4x: string;
  click_action?: string;
  click_url?: string;
}

interface RustUserBadge {
  badge_info: RustBadgeInfo;
  provider: 'twitch' | 'ffz' | 'bttv' | 'chatterino' | 'homies' | 'moltorino' | 'chatsen' | 'chatty' | 'dankchat';
}

interface RustUserBadgesResponse {
  display_badges: RustUserBadge[];
  earned_badges: RustUserBadge[];
  third_party_badges: RustUserBadge[];
}

/**
 * Transform a Rust badge to frontend format
 */
function transformBadge(rustBadge: RustUserBadge): TwitchBadge {
  const info = rustBadge.badge_info;
  return {
    id: info.id,
    setID: info.set_id,
    version: info.version,
    title: info.title,
    description: info.description,
    image1x: info.image_1x,
    image2x: info.image_2x,
    image4x: info.image_4x,
    clickAction: info.click_action,
    clickUrl: info.click_url,
  };
}

/**
 * Get all badges for a user (Twitch + Third-Party)
 * This version is optimized for chat - only fetches display badges
 * For profile overlays with full earned badge collection, use getAllUserBadgesWithEarned()
 */
export async function getAllUserBadges(
  userId: string,
  username: string,
  channelId: string,
  channelName: string
): Promise<UserBadgesResponse> {
  try {
    const rustResponse: RustUserBadgesResponse = await invoke('get_user_badges_unified', {
      userId,
      username,
      channelId,
      channelName,
    });

    // Transform to frontend format
    const displayBadges = (rustResponse.display_badges || []).map(transformBadge);
    const earnedBadges = (rustResponse.earned_badges || []).map(transformBadge);
    
    // Transform third-party badges (FFZ, Chatterino, Homies)
    const thirdPartyBadges: ThirdPartyBadge[] = (rustResponse.third_party_badges || []).map((b: RustUserBadge) => {
      // Use highest resolution available as default imageUrl
      const imageUrl = b.badge_info.image_4x || b.badge_info.image_2x || b.badge_info.image_1x;
      return {
        id: b.badge_info.id,
        title: b.badge_info.title,
        imageUrl,
        image1x: b.badge_info.image_1x || imageUrl,
        image2x: b.badge_info.image_2x || imageUrl,
        image4x: b.badge_info.image_4x || imageUrl,
        provider: b.provider,
        link: b.badge_info.click_url,
      };
    });

    return {
      displayBadges,
      earnedBadges,
      thirdPartyBadges,
      ivrBadges: [], // Legacy field, not used anymore
    };
  } catch (error) {
    Logger.error('[badgeService] Failed to get user badges:', error);
    return {
      displayBadges: [],
      earnedBadges: [],
      thirdPartyBadges: [],
      ivrBadges: [],
    };
  }
}

/**
 * Get all badges for a user with FULL earned badge collection (for profile overlays)
 * This makes additional GQL queries to fetch ALL earned badges (including global badge collection)
 * Use this for profile overlays where you need to show all badges the user has earned
 */
export async function getAllUserBadgesWithEarned(
  userId: string,
  username: string,
  channelId: string,
  channelName: string
): Promise<UserBadgesResponse> {
  try {
    const rustResponse: RustUserBadgesResponse = await invoke('get_user_badges_with_earned_unified', {
      userId,
      username,
      channelId,
      channelName,
    });

    // Transform to frontend format
    const displayBadges = (rustResponse.display_badges || []).map(transformBadge);
    const earnedBadges = (rustResponse.earned_badges || []).map(transformBadge);
    
    // Transform third-party badges (FFZ, Chatterino, Homies)
    const thirdPartyBadges: ThirdPartyBadge[] = (rustResponse.third_party_badges || []).map((b: RustUserBadge) => {
      // Use highest resolution available as default imageUrl
      const imageUrl = b.badge_info.image_4x || b.badge_info.image_2x || b.badge_info.image_1x;
      return {
        id: b.badge_info.id,
        title: b.badge_info.title,
        imageUrl,
        image1x: b.badge_info.image_1x || imageUrl,
        image2x: b.badge_info.image_2x || imageUrl,
        image4x: b.badge_info.image_4x || imageUrl,
        provider: b.provider,
        link: b.badge_info.click_url,
      };
    });

    return {
      displayBadges,
      earnedBadges,
      thirdPartyBadges,
      ivrBadges: [], // Legacy field, not used anymore
    };
  } catch (error) {
    Logger.error('[badgeService] Failed to get user badges with earned:', error);
    return {
      displayBadges: [],
      earnedBadges: [],
      thirdPartyBadges: [],
      ivrBadges: [],
    };
  }
}

/**
 * Resolve ONLY a user's real chat-client (third-party) badges — their actual
 * BTTV / FFZ / Chatterino / Homies / Chatsen / Chatty / DankChat badges — from
 * the prefetched provider databases. Cache-only on the Rust side (no Twitch
 * token, no network round-trip), so chat can call it once per chatter. Returns
 * only the third-party set; Twitch and 7TV badges are resolved on their own
 * paths. Does NOT include the BTTV Pro loyalty badge (that needs a per-user
 * live socket lookup and stays an opt-in identity badge).
 */
export async function getGlobalThirdPartyBadges(userId: string): Promise<ThirdPartyBadge[]> {
  try {
    const rustResponse: RustUserBadgesResponse = await invoke('get_third_party_badges_for_user_unified', {
      userId,
    });
    return (rustResponse.third_party_badges || []).map((b: RustUserBadge) => {
      const imageUrl = b.badge_info.image_4x || b.badge_info.image_2x || b.badge_info.image_1x;
      return {
        id: b.badge_info.id,
        title: b.badge_info.title,
        imageUrl,
        image1x: b.badge_info.image_1x || imageUrl,
        image2x: b.badge_info.image_2x || imageUrl,
        image4x: b.badge_info.image_4x || imageUrl,
        provider: b.provider,
        link: b.badge_info.click_url,
      };
    });
  } catch (error) {
    Logger.error('[badgeService] Failed to get global third-party badges:', error);
    return [];
  }
}

/**
 * Parse a badge string from IRC (e.g., "subscriber/12,premium/1")
 * Returns array of badge IDs
 */
export async function parseBadgeString(badgeString: string): Promise<string[]> {
  return await invoke('parse_badge_string', { badgeString });
}

/**
 * Pre-fetch global badges (optional, done automatically on startup)
 */
export async function prefetchGlobalBadges(): Promise<void> {
  await invoke('prefetch_global_badges_unified');
}

/**
 * Pre-fetch channel-specific badges
 */
export async function prefetchChannelBadges(channelId: string): Promise<void> {
  await invoke('prefetch_channel_badges_unified', { channelId });
}

/**
 * Pre-fetch third-party badge databases (FFZ, Chatterino, Homies)
 */
export async function prefetchThirdPartyBadges(): Promise<void> {
  await invoke('prefetch_third_party_badges');
}

/**
 * Clear all badge caches
 */
export async function clearBadgeCache(): Promise<void> {
  await invoke('clear_badge_cache_unified');
}

/**
 * Clear badge cache for a specific channel
 */
export async function clearChannelBadgeCache(channelId: string): Promise<void> {
  await invoke('clear_channel_badge_cache_unified', { channelId });
}

// Legacy type aliases for backwards compatibility
export type BadgeInfo = TwitchBadge;
export type UserBadge = TwitchBadge;
