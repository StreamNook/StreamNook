import { useEffect, useId, useMemo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useShallow } from 'zustand/react/shallow';
import { startTabLive, useTabLiveStore } from '../stores/tabLiveStore';
import { liveStats, sourceKey, type LiveRow, type LiveSource, type SourceLive } from '../utils/tabLive';
import { Logger } from '../utils/logger';

// The live dot on a chat tab, for every tab strip (MultiChat's tabs, the
// MultiNook chat switcher). Rust owns the answer (services/chat_dock_live.rs):
// this only declares the strip's channels and reads their rows back.

export interface TabLive {
  /** One row per source, in tab order. */
  stats: SourceLive[];
  /** The same rows by tab key (`makeKey`). */
  byKey: Map<string, SourceLive>;
}

export function useTabLive(surface: string, sources: LiveSource[]): TabLive {
  // Each mounted strip owns its own declaration, so one unmounting never
  // clears another's.
  const id = useId();
  const owner = `${surface}-${id}`;
  const sig = sources.map((s) => `${s.provider ?? 'twitch'}\n${s.channel}\n${s.channelId ?? ''}`).join('\t');
  useEffect(() => {
    startTabLive();
    const channels = sig
      ? sig.split('\t').map((row) => {
          const [provider, login, channelId] = row.split('\n');
          return { provider, login, channel_id: channelId || null };
        })
      : [];
    void invoke('set_tab_live_channels', { surface: owner, channels }).catch((e: unknown) =>
      Logger.warn('[TabLive] declare failed:', e),
    );
  }, [owner, sig]);
  useEffect(
    () => () => {
      void invoke('set_tab_live_channels', { surface: owner, channels: [] }).catch(() => {});
    },
    [owner],
  );

  // Only this strip's live fields, compared shallowly: another channel's
  // numbers re-render nothing here.
  const keys = useMemo(() => sources.map(sourceKey), [sources]);
  const fields = useTabLiveStore(
    useShallow((s) =>
      keys.flatMap((k) => {
        const row = s.live[k];
        return [row ? row.live : null, row?.viewer_count ?? null];
      }),
    ),
  );
  return useMemo(() => {
    const rows: Record<string, LiveRow> = {};
    keys.forEach((k, i) => {
      const live = fields[2 * i];
      if (live !== null) rows[k] = { live: live === true, viewer_count: fields[2 * i + 1] as number | null };
    });
    const stats = liveStats(sources, rows);
    return { stats, byKey: new Map(stats.map((s) => [s.key, s])) };
  }, [sources, keys, fields]);
}
