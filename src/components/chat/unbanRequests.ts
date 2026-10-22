// Unban requests a channel's moderators can answer, live from Rust
// (`get_unban_requests`, pushed as `unban-requests`). Shared by the desktop
// strip and the phones' Moderation sheet.

import { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Logger } from '../../utils/logger';

export interface UnbanRequest {
  id: string;
  /** Lowercase broadcaster login. */
  channel: string;
  broadcaster_id: string;
  user_id: string;
  user_login: string;
  user_name: string;
  /** What the chatter wrote. */
  text: string;
  created_at_ms: number;
  /** How long ago, in Rust's words: "4 minutes ago". */
  age: string;
}

interface UnbanPush {
  channel: string;
  requests: UnbanRequest[];
}

/** Twitch's limit on the reply the chatter sees. */
export const UNBAN_REPLY_MAX = 500;

/** How often an open pane asks again, so ages stay current. */
const REFRESH_MS = 60_000;

/** One channel's rows out of whatever the hook last heard. */
export function rowsFor(rows: UnbanRequest[], channel: string): UnbanRequest[] {
  const key = channel.toLowerCase();
  return rows.filter((r) => r.channel === key);
}

/** The collapsed line: "2 unban requests · Gus: I was joking". */
export function summaryLine(rows: UnbanRequest[], open: boolean): string {
  const noun = rows.length === 1 ? 'unban request' : 'unban requests';
  const first = rows[0];
  if (open || !first) return noun;
  return `${noun} · ${first.user_name || first.user_login}: ${first.text}`;
}

/** The channel's pending unban requests, live, and the way to answer one. */
export function useUnbanRequests(channelId: string, channel: string, enabled = true) {
  const key = channel.toLowerCase();
  const [heard, setHeard] = useState<UnbanRequest[]>([]);

  useEffect(() => {
    if (!enabled || !key) return;
    let alive = true;
    const load = () => {
      invoke<UnbanRequest[]>('get_unban_requests', { channelId, channel: key })
        .then((rows) => {
          if (alive) setHeard(rows);
        })
        .catch((err) => Logger.debug('[Unban requests] read failed:', err));
    };
    load();
    const off = listen<UnbanPush>('unban-requests', (e) => {
      if (alive && e.payload.channel === key) setHeard(e.payload.requests);
    });
    const timer = window.setInterval(load, REFRESH_MS);
    return () => {
      alive = false;
      window.clearInterval(timer);
      void off.then((f) => f());
    };
  }, [channelId, key, enabled]);

  const rows = useMemo(() => (enabled ? rowsFor(heard, key) : []), [enabled, heard, key]);

  const resolve = useCallback(
    async (requestId: string, approve: boolean, reply: string): Promise<string | null> => {
      try {
        await invoke('resolve_unban_request', {
          channelId,
          requestId,
          approve,
          text: reply.trim() ? reply : null,
        });
        return null;
      } catch (err) {
        return typeof err === 'string' ? err : 'That did not go through';
      }
    },
    [channelId],
  );

  return { rows, resolve };
}
