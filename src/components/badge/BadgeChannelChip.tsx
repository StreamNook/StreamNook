import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { TwitchStream } from '../../types';
import { Tooltip } from '../ui/Tooltip';
import { resolveAvatar, DEFAULT_AVATAR } from '../multi-nook/channelSearch';
import { Logger } from '../../utils/logger';

interface Resolved {
  login: string;
  displayName: string;
  avatarUrl?: string;
  isLive: boolean;
}

// Cache resolutions so repeated chips (and panel reopens) don't re-hit search.
const cache = new Map<string, Resolved | null>();

async function resolveChannel(login: string): Promise<Resolved | null> {
  const key = login.toLowerCase();
  const cached = cache.get(key);
  if (cached !== undefined) return cached;
  try {
    const results = (await invoke('search_channels', { query: login })) as TwitchStream[];
    const hit = results.find((r) => (r.user_login || '').toLowerCase() === key);
    const resolved: Resolved | null = hit
      ? {
          login: hit.user_login || login,
          displayName: hit.user_name || login,
          avatarUrl: resolveAvatar(hit.profile_image_url, hit.thumbnail_url),
          isLive: !!hit.is_live,
        }
      : null;
    cache.set(key, resolved);
    return resolved;
  } catch (err) {
    Logger.warn('[BadgeChannelChip] resolve failed:', err);
    return null;
  }
}

/** Inline clickable chip for a channel named in badge earn text (e.g.
 *  "/studbudz"). Resolves the avatar + display name, shows a live dot, and
 *  opens the channel on click. Falls back to a plain "/login" span until (or if
 *  never) the channel resolves, so the text always reads correctly. */
export const BadgeChannelChip = ({
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

  if (!data) {
    return <span className="text-textSecondary">/{login}</span>;
  }

  return (
    <Tooltip content={`Watch ${data.displayName}`}>
      <button
        type="button"
        onClick={() => onWatch(data.login)}
        style={{ verticalAlign: '-5px' }}
        className="inline-flex items-center gap-1.5 pl-0.5 pr-2 py-0.5 rounded-full bg-white/[0.06] hover:bg-white/[0.10] transition-colors"
      >
        <img
          src={data.avatarUrl || DEFAULT_AVATAR}
          alt=""
          onError={(e) => {
            const img = e.currentTarget as HTMLImageElement;
            if (img.src !== DEFAULT_AVATAR) img.src = DEFAULT_AVATAR;
          }}
          className="w-[18px] h-[18px] rounded-full object-cover"
        />
        <span className="text-[13px] font-medium text-textPrimary leading-none">{data.displayName}</span>
        {data.isLive && <span className="w-1.5 h-1.5 rounded-full bg-red-500 shrink-0" aria-label="live" />}
      </button>
    </Tooltip>
  );
};
