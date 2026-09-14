import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowUpRight } from 'lucide-react';
import type { TwitchStream } from '../../types';
import { resolveAvatar, DEFAULT_AVATAR } from '../multi-nook/channelSearch';
import { Logger } from '../../utils/logger';

interface Resolved {
  login: string;
  displayName: string;
  avatarUrl?: string;
  isLive: boolean;
  gameName?: string;
  viewers?: number;
  // The live stream object, handed to startStream so the channel opens with all
  // the normal services (player, chat, metadata).
  stream?: TwitchStream;
}

const cache = new Map<string, Resolved | null>();

async function resolveChannel(login: string): Promise<Resolved | null> {
  const key = login.toLowerCase();
  const cached = cache.get(key);
  if (cached !== undefined) return cached;
  try {
    const results = (await invoke('search_channels', { query: login })) as TwitchStream[];
    const hit = results.find((r) => (r.user_login || '').toLowerCase() === key);
    if (!hit) {
      cache.set(key, null);
      return null;
    }
    // Reliable live data (viewers + category) from a direct stream check.
    let live: TwitchStream | null = null;
    try {
      live = (await invoke('check_stream_online', {
        userLogin: hit.user_login || login,
      })) as TwitchStream | null;
    } catch {
      /* offline or lookup failed */
    }
    const resolved: Resolved = {
      login: hit.user_login || login,
      displayName: hit.user_name || login,
      avatarUrl: resolveAvatar(hit.profile_image_url, hit.thumbnail_url),
      isLive: !!live,
      gameName: live?.game_name ?? undefined,
      viewers: live?.viewer_count ?? undefined,
      stream: live ?? undefined,
    };
    cache.set(key, resolved);
    return resolved;
  } catch (err) {
    Logger.warn('[BadgeChannelCard] resolve failed:', err);
    return null;
  }
}

/** A themed card for a streamer a badge points at: avatar, name, and a live line
 *  (category + viewer count) or "Offline". Clicking opens the channel through the
 *  app's normal watch flow (`onWatch`). Renders nothing until resolved, or if the
 *  channel can't be found. */
export const BadgeChannelCard = ({
  login,
  onWatch,
}: {
  login: string;
  onWatch: (login: string, streamInfo?: TwitchStream) => void;
}) => {
  const [data, setData] = useState<Resolved | null | undefined>(() => cache.get(login.toLowerCase()));

  useEffect(() => {
    let alive = true;
    if (cache.get(login.toLowerCase()) === undefined) {
      resolveChannel(login).then((r) => {
        if (alive) setData(r);
      });
    }
    return () => {
      alive = false;
    };
  }, [login]);

  if (!data) return null;

  return (
    <button
      onClick={() => onWatch(data.login, data.stream)}
      className="group flex items-center gap-3 w-full text-left p-2.5 rounded-xl bg-white/[0.04] hover:bg-white/[0.07] border border-white/[0.06] transition-colors"
    >
      <div className="relative shrink-0">
        <img
          src={data.avatarUrl || DEFAULT_AVATAR}
          alt=""
          onError={(e) => {
            const img = e.currentTarget as HTMLImageElement;
            if (img.src !== DEFAULT_AVATAR) img.src = DEFAULT_AVATAR;
          }}
          className="w-[46px] h-[46px] rounded-full object-cover"
        />
        {data.isLive && (
          <span className="absolute -bottom-0.5 -right-0.5 w-3 h-3 rounded-full bg-red-500 border-2 border-secondary" />
        )}
      </div>
      <div className="flex-1 min-w-0">
        <div className="text-[11px] text-textMuted uppercase tracking-wide">Channel</div>
        <div className="text-[15px] font-medium text-textPrimary truncate group-hover:text-accent transition-colors">
          {data.displayName}
        </div>
        <div className="text-[12px] truncate">
          {data.isLive ? (
            <span className="text-red-400">
              Live
              {data.gameName ? ` · ${data.gameName}` : ''}
              {typeof data.viewers === 'number' ? ` · ${data.viewers.toLocaleString()} watching` : ''}
            </span>
          ) : (
            <span className="text-textSecondary">Offline</span>
          )}
        </div>
      </div>
      <ArrowUpRight
        size={18}
        className="text-textMuted group-hover:text-accent transition-colors shrink-0"
      />
    </button>
  );
};
