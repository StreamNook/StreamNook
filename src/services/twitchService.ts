import { invoke } from '@tauri-apps/api/core';
import { Logger } from '../utils/logger';
// Service for Twitch API calls (GraphQL and Helix)

export interface MySubscription {
  channelId: string;
  channelLogin: string;
  channelDisplayName: string;
  tier: 1 | 2 | 3;
  isPrime: boolean;
  isGift: boolean;
}

/**
 * Fetch the current user's active subscriptions. Delegates to the Rust
 * `get_my_subscriptions` command, which queries `currentUser.subscriptionBenefits`
 * through the same first-party auth path the resub feature uses (Android
 * client-id + OAuth token from the drops auth service). `currentUser` does not
 * resolve with the Helix client-id / token available in the frontend. Returns
 * [] on failure so callers degrade gracefully.
 */
export async function fetchMySubscriptions(): Promise<MySubscription[]> {
  try {
    return await invoke<MySubscription[]>('get_my_subscriptions');
  } catch (error) {
    Logger.warn('[Subscriptions] get_my_subscriptions failed:', error);
    return [];
  }
}

export interface PastSubscription {
  channelId: string;
  channelLogin: string;
  channelDisplayName: string;
  tier: 1 | 2 | 3;
  // Months of tenure for that (now-ended) subscription period.
  months: number;
}

/**
 * Fetch the current user's expired / past subscriptions (the `get_my_past_subscriptions`
 * Rust command, querying `currentUser.expiredSubscriptions`). Returns [] on failure.
 */
export async function fetchMyPastSubscriptions(): Promise<PastSubscription[]> {
  try {
    return await invoke<PastSubscription[]>('get_my_past_subscriptions');
  } catch (error) {
    Logger.warn('[Subscriptions] get_my_past_subscriptions failed:', error);
    return [];
  }
}

