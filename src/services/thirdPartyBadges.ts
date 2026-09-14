import { Logger } from '../utils/logger';
/**
 * DEPRECATED: This service has been replaced by the unified Rust badge service
 * 
 * Third-party badge functionality (FFZ, Chatterino, Homies) is now handled in Rust
 * for maximum performance. All badges are automatically fetched and cached by the
 * Rust backend.
 * 
 * Please use `badgeService.ts` instead:
 * 
 * ```typescript
 * import { getAllUserBadges, prefetchThirdPartyBadges } from './badgeService';
 * 
 * // Get all badges (including third-party) for a user
 * const badges = await getAllUserBadges(userId, username, channelId, channelName);
 * Logger.debug(badges.third_party_badges); // FFZ, Chatterino, Homies badges
 * ```
 * 
 * The Rust backend handles:
 * - Global badge database caching (10 minute expiry)
 * - Automatic fetching from FFZ, Chatterino, and Homies APIs
 * - Efficient lookups by user ID
 * - Background pre-fetching on app startup
 */

// Re-export from unified service for backwards compatibility
export {
  getAllUserBadges,
  prefetchThirdPartyBadges,
  clearBadgeCache,
  type ThirdPartyBadge,
  type UserBadgesResponse,
} from './badgeService';

/**
 * @deprecated Use getAllUserBadges() instead
 * This function is no longer needed as third-party badges are included automatically
 */

/**
 * @deprecated Use clearBadgeCache() from badgeService.ts
 */
export function clearThirdPartyBadgeCaches(): void {
  Logger.warn('[thirdPartyBadges] clearThirdPartyBadgeCaches() is deprecated. Use clearBadgeCache() from badgeService.ts');
  // No-op, use clearBadgeCache() instead
}
