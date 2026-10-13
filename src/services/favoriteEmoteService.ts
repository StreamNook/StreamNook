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
 * The favorites this chat offers, in favorite order, each drawn from the
 * chat's own entry for that emote. A saved favorite is a copy from when it was
 * starred and can lack what the picker learned since (whether it animates,
 * its cached file), so only `addedAt` comes from the copy.
 */
export function getAvailableFavorites(allEmotes: Emote[]): FavoriteEmote[] {
  if (!favoriteEmotesCache) {
    return [];
  }

  const byKey = new Map<string, Emote>();
  const byId = new Map<string, Emote>();
  for (const e of allEmotes) {
    byKey.set(`${e.provider}:${e.id}`, e);
    if (!byId.has(e.id)) byId.set(e.id, e);
  }

  return favoriteEmotesCache.flatMap((fav) => {
    const live = byKey.get(`${fav.provider}:${fav.id}`) ?? byId.get(fav.id);
    return live ? [{ ...fav, ...live, addedAt: fav.addedAt }] : [];
  });
}
