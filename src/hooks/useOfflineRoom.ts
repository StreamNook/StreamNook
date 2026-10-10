import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { OfflineRoom } from '../types';

/** A Twitch channel's offline room (Rust `open_offline_room`, cached there),
 *  for a surface that shows the offline card outside the main view: an
 *  offline MultiNook tile. `null` login loads nothing. */
export function useOfflineRoom(login: string | null): OfflineRoom | null {
  const [loaded, setLoaded] = useState<{ login: string; room: OfflineRoom } | null>(null);
  useEffect(() => {
    if (!login) return;
    let alive = true;
    invoke<OfflineRoom>('open_offline_room', { login })
      .then((room) => {
        if (alive) setLoaded({ login, room });
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [login]);
  return loaded && loaded.login === login ? loaded.room : null;
}
