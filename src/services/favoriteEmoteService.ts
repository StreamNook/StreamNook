// Service for managing favorite emotes
import { invoke } from '@tauri-apps/api/core';
import { Emote } from './emoteService';

import { Logger } from '../utils/logger';
export interface FavoriteEmote extends Emote {
  addedAt: number; // Timestamp when added to favorites
}

// In-memory cache for quick access
let favoriteEmotesCache: FavoriteEmote[] | null = null;

/**
 * Load favorite emotes from cache
 */
export async function loadFavoriteEmotes(): Promise<FavoriteEmote[]> {
  try {
    const cached = await invoke<string | null>('load_favorite_emotes_cache');
    
    if (cached) {
      const favorites = JSON.parse(cached) as FavoriteEmote[];
      favoriteEmotesCache = favorites;
      return favorites;
    }
    
    favoriteEmotesCache = [];
    return [];
  } catch (error) {
    Logger.error('[FavoriteEmoteService] Failed to load favorite emotes:', error);
    favoriteEmotesCache = [];
    return [];
  }
}

/**
 * Add an emote to favorites
 */
export async function addFavoriteEmote(emote: Emote): Promise<void> {
  try {
    const favoriteEmote: FavoriteEmote = {
      ...emote,
      addedAt: Date.now()
    };
    
    const data = JSON.stringify(favoriteEmote);
    await invoke('add_favorite_emote_cache', { emoteData: data });
    
    // Update in-memory cache
    if (favoriteEmotesCache) {
      // Check if already exists
      const exists = favoriteEmotesCache.some(e => e.id === emote.id);
      if (!exists) {
        favoriteEmotesCache.push(favoriteEmote);
      }
    } else {
      // Load cache if not loaded yet
      await loadFavoriteEmotes();
    }
  } catch (error) {
    Logger.error('[FavoriteEmoteService] Failed to add favorite emote:', error);
    throw error;
  }
}

/**
 * Remove an emote from favorites
 */
export async function removeFavoriteEmote(emoteId: string): Promise<void> {
  try {
    await invoke('remove_favorite_emote_cache', { emoteId });
    
    // Update in-memory cache
    if (favoriteEmotesCache) {
      favoriteEmotesCache = favoriteEmotesCache.filter(e => e.id !== emoteId);
    }
  } catch (error) {
    Logger.error('[FavoriteEmoteService] Failed to remove favorite emote:', error);
    throw error;
  }
}

/**
 * Check if an emote is favorited
 */
export function isFavoriteEmote(emoteId: string): boolean {
  if (!favoriteEmotesCache) {
    return false;
  }
  
  return favoriteEmotesCache.some(e => e.id === emoteId);
}

/**
 * Get all favorite emotes (from cache)
 */
export function getFavoriteEmotes(): FavoriteEmote[] {
  return favoriteEmotesCache || [];
}

/**
 * Filter favorite emotes by availability in current emote set
 * Returns favorites that are available in the provided emote set
 */
export function getAvailableFavorites(allEmotes: Emote[]): FavoriteEmote[] {
  if (!favoriteEmotesCache) {
    return [];
  }
  
  // Create a Set of available emote IDs for fast lookup
  const availableEmoteIds = new Set(allEmotes.map(e => e.id));
  
  // Filter favorites to only those available in current chat
  return favoriteEmotesCache.filter(fav => availableEmoteIds.has(fav.id));
}
