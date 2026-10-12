import { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useShallow } from 'zustand/react/shallow';
import type { ProviderId } from '../../types/providers';
import { useVisibleInterval } from '../../utils/useVisibleInterval';
import { unwatchChannel, useChannelStateStore, watchChannel } from '../../stores/channelStateStore';
import {
  liveStats,
  sourceKey,
  twitchStateLogin,
  type LiveSource,
  type ProviderLiveMeta,
  type SourceLive,
  type TwitchLiveState,
} from './multiChatLive';

// One reader of live state per MultiChat window, shared by the title bar's
// viewer total and the tab strip's live dots. It asks nothing of any platform
// itself. Twitch sources are watched in Rust's channel state for as long as
// they are open (the watch also covers blended mode, which mounts no panes);
// every other platform is read from the meta its chat adapter already holds.

const PROVIDER_READ_MS = 45_000;

function metaCommandFor(provider: ProviderId): string | null {
  if (provider === 'kick') return 'get_kick_channel_meta';
  if (provider === 'youtube') return 'get_youtube_channel_meta';
  if (provider === 'tiktok') return 'get_tiktok_channel_meta';
  return null;
}

async function readProviderMeta(src: LiveSource): Promise<ProviderLiveMeta | null> {
  const cmd = metaCommandFor(src.provider ?? 'twitch');
  if (!cmd) return null;
  try {
    return await invoke<ProviderLiveMeta | null>(cmd, { slug: src.channel.toLowerCase() });
  } catch {
    return null;
  }
}

export interface MultiChatLive {
  /** One row per source, in tab order. */
  stats: SourceLive[];
  /** The same rows by tab key (`makeKey`). */
  byKey: Map<string, SourceLive>;
}

export function useMultiChatLive(sources: LiveSource[]): MultiChatLive {
  // Twitch: one watch per open channel. A channel whose id has not resolved
  // yet joins once it has.
  const twitchWatch = useMemo(
    () =>
      sources
        .filter((c) => (c.provider ?? 'twitch') === 'twitch' && c.channelId)
        .map((c) => `${twitchStateLogin(c)} ${c.channelId}`)
        .join(','),
    [sources],
  );
  useEffect(() => {
    const pairs = twitchWatch ? twitchWatch.split(',').map((p) => p.split(' ')) : [];
    for (const [login, id] of pairs) void watchChannel(login, id);
    return () => {
      for (const [login] of pairs) void unwatchChannel(login);
    };
  }, [twitchWatch]);
  // Only the live fields of this window's own Twitch sources, compared
  // shallowly. This hook runs in the window itself, so a points, pin or collab
  // update, or another channel's numbers, must re-render nothing.
  const twitchLogins = useMemo(
    () => sources.filter((c) => (c.provider ?? 'twitch') === 'twitch').map(twitchStateLogin),
    [sources],
  );
  const twitchFields = useChannelStateStore(
    useShallow((s) =>
      twitchLogins.flatMap((login) => {
        const state = s.channels.get(login);
        return [state?.viewer_count ?? null, state?.viewers_at ?? null];
      }),
    ),
  );
  const twitchStates = useMemo(() => {
    const states = new Map<string, TwitchLiveState>();
    twitchLogins.forEach((login, i) => {
      states.set(login, { viewer_count: twitchFields[2 * i], viewers_at: twitchFields[2 * i + 1] });
    });
    return states;
  }, [twitchLogins, twitchFields]);

  // Everything else: each source's answer lands on its own, so a slow one
  // never holds back the rest.
  const [providerMeta, setProviderMeta] = useState<Record<string, ProviderLiveMeta | null>>({});
  const providerSources = useMemo(
    () => sources.filter((c) => metaCommandFor(c.provider ?? 'twitch') !== null),
    [sources],
  );
  const readOne = useCallback((src: LiveSource) => {
    const key = sourceKey(src);
    void readProviderMeta(src).then((m) => {
      const meta = m ? { is_live: m.is_live === true, viewer_count: m.viewer_count ?? null } : null;
      setProviderMeta((prev) => {
        const old = prev[key];
        const same =
          old === meta ||
          (old != null && meta != null && old.is_live === meta.is_live && old.viewer_count === meta.viewer_count);
        return same && key in prev ? prev : { ...prev, [key]: meta };
      });
    });
  }, []);
  const readProviders = useCallback(() => {
    for (const src of providerSources) readOne(src);
  }, [providerSources, readOne]);
  useEffect(() => {
    readProviders();
  }, [readProviders]);
  useVisibleInterval(readProviders, PROVIDER_READ_MS);

  // A pane announces when its adapter has resolved the channel; read that
  // source then instead of waiting out the interval, so a freshly added tab
  // shows its state as soon as Rust knows it.
  useEffect(() => {
    const onResolved = (e: Event) => {
      const detail = (e as CustomEvent<{ provider?: ProviderId; channel?: string }>).detail;
      if (!detail?.provider || !detail.channel) return;
      const key = sourceKey({ provider: detail.provider, channel: detail.channel });
      const src = providerSources.find((s) => sourceKey(s) === key);
      if (src) readOne(src);
    };
    window.addEventListener('multichat-source-resolved', onResolved);
    return () => window.removeEventListener('multichat-source-resolved', onResolved);
  }, [providerSources, readOne]);

  return useMemo(() => {
    const stats = liveStats(sources, twitchStates, providerMeta);
    return { stats, byKey: new Map(stats.map((s) => [s.key, s])) };
  }, [sources, twitchStates, providerMeta]);
}
