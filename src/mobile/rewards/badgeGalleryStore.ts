// The badge wall, held OUTSIDE the Rewards screen.
//
// The tab shell unmounts a screen the moment you leave it; kept here, a
// revisit renders what was on screen a moment ago while Rust is asked again.
// The wall itself (catalogue merged by title, ownership, earn status, the
// sort, the sheet copy, the missing strip, the once-per-session metadata
// backfill and the timer for the next earn-window boundary) is built in Rust,
// services/badge_wall.rs; this store only holds the last answer.
import { create } from 'zustand';
import { listen } from '@tauri-apps/api/event';
import { getBadgeWall, type BadgeSort, type BadgeWall } from './rewardsModel';
import { Logger } from '../../utils/logger';

interface BadgeGalleryState {
  wall: BadgeWall | null;
  loading: boolean;
  /** Badges whose metadata Rust is still fetching; 0 when idle. */
  syncLeft: number;
}

export const useBadgeGallery = create<BadgeGalleryState>(() => ({
  wall: null,
  loading: false,
  syncLeft: 0,
}));

/** True when the wall can render straight from memory. */
export function badgeGalleryIsWarm(): boolean {
  return (useBadgeGallery.getState().wall?.tiles.length ?? 0) > 0;
}

let request = 0;

/**
 * Ask Rust for the wall in `sort`. `force` also re-reads the account's
 * collection in the background (`badge-standing-changed` follows). Only the
 * newest request applies; a failed read keeps the wall on screen.
 */
export async function loadBadgeGallery(sort: BadgeSort, force = false): Promise<void> {
  const mine = ++request;
  useBadgeGallery.setState({ loading: true });
  try {
    const wall = await getBadgeWall(sort, force);
    if (mine === request) useBadgeGallery.setState({ wall, syncLeft: wall.sync_left });
  } catch (err) {
    Logger.warn('[Rewards] badge wall unavailable:', err);
  } finally {
    if (mine === request) useBadgeGallery.setState({ loading: false });
  }
}

let syncListening = false;

/** The backfill's countdown, for the "<n> to sync" line. */
export function followBadgeSync(): void {
  if (syncListening) return;
  syncListening = true;
  void listen<{ left: number }>('badge-wall-sync', (event) => {
    useBadgeGallery.setState({ syncLeft: event.payload.left });
  });
}
