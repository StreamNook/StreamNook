import { useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import { useAppStore } from '../stores/AppStore';
import { incrementStat } from '../services/supabaseService';
import { Logger } from '../utils/logger';

/**
 * Count every bonus chest Rust collects on a watched channel toward the
 * profile's channel-points stat. Rust claims the chest whichever chat is on
 * screen (services/watched_chest.rs), so the stat cannot live in a chat pane.
 *
 * Mount ONCE, in the main window only: every window hears the event, and a
 * second listener would count each chest twice.
 */
export function useWatchedChestStat(): void {
  useEffect(() => {
    const unlisten = listen<{ channel_id: string; points_earned: number }>('watched-chest-claimed', (event) => {
      const userId = useAppStore.getState().currentUser?.user_id;
      const earned = event.payload.points_earned;
      if (!userId || earned <= 0) return;
      incrementStat(userId, 'channel_points_collected', earned).catch((err) =>
        Logger.warn('[WatchedChest] channel points stat failed:', err),
      );
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, []);
}
