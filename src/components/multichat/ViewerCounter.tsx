import { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { createPortal } from 'react-dom';
import { UsersThree } from 'phosphor-react';
import { ProviderLogo } from '../ProviderLogo';
import type { ProviderId } from '../../types/providers';
import { useVisibleInterval } from '../../utils/useVisibleInterval';
import { unwatchChannel, useChannelStateStore, watchChannel } from '../../stores/channelStateStore';

// A clean, aggregate viewer counter for the MultiChat title bar: total live
// viewers across every open source, with a per-stream breakdown on hover. It
// asks nothing of any platform itself. Twitch counts come from Rust's channel
// state, one batched poll for every watched channel that the chat panes share;
// the counter holds its own watches because blended mode mounts no panes.
// Every other platform's count is already held by its chat adapter in Rust
// and is read from there, each source on its own.

interface ViewerSource {
  channel: string;
  channelName?: string;
  provider?: ProviderId;
  /** Twitch user id, which a channel-state watch is keyed by. */
  channelId?: string | null;
}

interface ViewerStat {
  key: string;
  name: string;
  provider: ProviderId;
  count: number | null;
  isLive: boolean;
}

const VIEWER_POLL_MS = 45_000;

function metaCommandFor(provider: ProviderId): string | null {
  if (provider === 'kick') return 'get_kick_channel_meta';
  if (provider === 'youtube') return 'get_youtube_channel_meta';
  if (provider === 'tiktok') return 'get_tiktok_channel_meta';
  return null;
}

/** A non-Twitch source's count, read from what its chat adapter holds. */
async function fetchStat(src: ViewerSource): Promise<ViewerStat> {
  const provider = src.provider ?? 'twitch';
  const slug = src.channel.toLowerCase();
  const base = { key: `${provider}:${slug}`, name: src.channelName || src.channel, provider };
  try {
    const cmd = metaCommandFor(provider);
    if (cmd) {
      const m = await invoke<{ viewer_count?: number | null; is_live?: boolean } | null>(cmd, {
        slug,
      });
      return { ...base, count: m?.viewer_count ?? null, isLive: m?.is_live ?? false };
    }
  } catch {
    /* leave unknown */
  }
  return { ...base, count: null, isLive: false };
}

export default function ViewerCounter({ channels }: { channels: ViewerSource[] }) {
  const [providerStats, setProviderStats] = useState<Record<string, ViewerStat>>({});
  const [anchor, setAnchor] = useState<{ top: number; left: number } | null>(null);

  // Twitch: watch each channel in Rust's channel state for as long as it is
  // here. A channel whose id has not resolved yet joins once it has.
  const twitchWatch = useMemo(
    () =>
      channels
        .filter((c) => (c.provider ?? 'twitch') === 'twitch' && c.channelId)
        .map((c) => `${c.channel.toLowerCase()} ${c.channelId}`)
        .join(','),
    [channels],
  );
  useEffect(() => {
    const pairs = twitchWatch ? twitchWatch.split(',').map((p) => p.split(' ')) : [];
    for (const [login, id] of pairs) void watchChannel(login, id);
    return () => {
      for (const [login] of pairs) void unwatchChannel(login);
    };
  }, [twitchWatch]);
  const twitchStates = useChannelStateStore((s) => s.channels);

  // Everything else: each source's count lands on its own, so a slow one never
  // holds back the rest.
  const providerSources = useMemo(
    () => channels.filter((c) => (c.provider ?? 'twitch') !== 'twitch'),
    [channels],
  );
  const readProviders = useCallback(() => {
    for (const src of providerSources) {
      void fetchStat(src).then((stat) => setProviderStats((prev) => ({ ...prev, [stat.key]: stat })));
    }
  }, [providerSources]);
  useEffect(() => {
    readProviders();
  }, [readProviders]);
  useVisibleInterval(readProviders, VIEWER_POLL_MS);

  const stats = useMemo<ViewerStat[]>(
    () =>
      channels.map((src) => {
        const provider = src.provider ?? 'twitch';
        const slug = src.channel.toLowerCase();
        const key = `${provider}:${slug}`;
        const name = src.channelName || src.channel;
        if (provider === 'twitch') {
          // Rust reports no count for a channel that is not live.
          const count = twitchStates.get(slug)?.viewer_count ?? null;
          return { key, name, provider, count, isLive: count !== null };
        }
        return providerStats[key] ?? { key, name, provider, count: null, isLive: false };
      }),
    [channels, twitchStates, providerStats],
  );

  const total = useMemo(
    () => stats.reduce((sum, s) => sum + (s.isLive && s.count ? s.count : 0), 0),
    [stats],
  );
  const liveCount = stats.filter((s) => s.isLive).length;

  if (channels.length === 0) return null;

  return (
    <>
      <button
        type="button"
        data-tauri-drag-region="false"
        onMouseEnter={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          setAnchor({ top: r.bottom + 6, left: r.left });
        }}
        onMouseLeave={() => setAnchor(null)}
        className="flex items-center gap-1.5 rounded px-2 py-0.5 text-xs text-textSecondary transition-colors hover:bg-white/5 hover:text-textPrimary"
      >
        <UsersThree size={13} weight="fill" />
        <span className="font-medium tabular-nums">{total.toLocaleString()}</span>
        {liveCount > 0 && (
          <span className="relative flex h-1.5 w-1.5">
            <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-red-500 opacity-60" />
            <span className="relative inline-flex h-1.5 w-1.5 rounded-full bg-red-500" />
          </span>
        )}
      </button>
      {anchor &&
        createPortal(
          <div
            className="glass-panel fixed z-[300] min-w-[190px] rounded-lg border border-borderLight p-1.5 shadow-xl"
            // Opaque themed surface (not the translucent glass default): over the
            // scrolling chat a live backdrop-blur flickers in WebView2.
            style={{ top: anchor.top, left: anchor.left, backgroundColor: 'var(--color-background-tertiary)', backdropFilter: 'none', WebkitBackdropFilter: 'none' }}
          >
            <div className="px-1.5 pb-1 pt-0.5 text-[10px] font-semibold uppercase tracking-wide text-textMuted">
              Viewers{liveCount > 0 ? ` · ${total.toLocaleString()} watching` : ''}
            </div>
            {stats.map((s) => (
              <div key={s.key} className="flex items-center gap-2 rounded px-1.5 py-1 text-xs">
                <ProviderLogo provider={s.provider} size={13} />
                <span className="min-w-0 flex-1 truncate text-textSecondary">{s.name}</span>
                {s.isLive ? (
                  <span className="font-medium tabular-nums text-textPrimary">
                    {(s.count ?? 0).toLocaleString()}
                  </span>
                ) : (
                  <span className="text-[10px] uppercase tracking-wide text-textMuted">offline</span>
                )}
              </div>
            ))}
          </div>,
          document.body,
        )}
    </>
  );
}
