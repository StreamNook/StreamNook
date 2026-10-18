// Pick another channel's chat to open in a tab.
//
// Live follows come first because that is where moderating usually happens, and
// they need no network call. Search covers everyone else, and deliberately
// includes offline channels: chat is joinable whether or not the channel is
// streaming, which is exactly the case for moderating a friend's quiet room.
// Rust answers the search (`search_chat_rooms`: Twitch and Kick together, one
// failing never emptying the other, live rooms first) and reads pasted Kick
// links (`chat_room_from_link`), the same answers the iPhone app shows.
import React, { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { MagnifyingGlass, UsersThree } from 'phosphor-react';
import { MobileSheet } from '../ui/MobileSheet';
import { useAppStore } from '../../stores/AppStore';
import { useChatTabsStore } from './chatTabsStore';
import { Logger } from '../../utils/logger';
import type { TwitchStream } from '../../types';
import type { ProviderId } from '../../types/providers';
import { ProviderMark } from '../../components/ProviderLogo';
import { isTwitchStream, streamKey, streamProvider } from '../../utils/streamProvider';
import { phoneFollowedLive } from '../followingMerge';
import { SevenTvAvatar } from '../../components/SevenTvAvatar';
import { sevenTvIdForStream } from '../../services/sevenTvPictures';

/** A channel as `search_chat_rooms` and `chat_room_from_link` answer it. */
interface ChatRoomRow {
  provider: ProviderId;
  key: string;
  user_login: string;
  user_id: string;
  user_name: string;
  profile_image_url: string | null;
  is_live: boolean;
  viewer_count: number;
  game_name: string;
  pasted: boolean;
}

/** The row in the shape the list draws, beside the live follows. */
function asStream(row: ChatRoomRow): TwitchStream {
  return {
    id: row.key,
    user_id: row.user_id,
    user_login: row.user_login,
    user_name: row.user_name,
    title: '',
    viewer_count: row.viewer_count,
    game_name: row.game_name,
    thumbnail_url: '',
    started_at: '',
    profile_image_url: row.profile_image_url ?? undefined,
    provider: row.provider,
    is_live: row.is_live,
  } as TwitchStream;
}

export const AddChatSheet: React.FC<{ open: boolean; onClose: () => void }> = ({
  open,
  onClose,
}) => {
  const openTabs = useChatTabsStore((s) => s.tabs);
  const addTab = useChatTabsStore((s) => s.addTab);

  const [query, setQuery] = useState('');
  const [results, setResults] = useState<TwitchStream[]>([]);
  const [searching, setSearching] = useState(false);
  // A pasted kick.com link (or `kick:slug`) opens that room directly, live or
  // not, without a search round trip.
  const [pasted, setPasted] = useState<TwitchStream | null>(null);

  // Live Kick follows sit beside the Twitch ones (merged in Rust).
  const following = useAppStore((s) => s.following);
  const liveNow = useMemo(() => phoneFollowedLive(following), [following]);

  const openSet = useMemo(
    () => new Set(openTabs.map((t) => t.channel)),
    [openTabs],
  );

  useEffect(() => {
    let cancelled = false;
    void invoke<ChatRoomRow | null>('chat_room_from_link', { query })
      .then((row) => {
        if (!cancelled) setPasted(row ? asStream(row) : null);
      })
      .catch(() => {
        if (!cancelled) setPasted(null);
      });
    return () => {
      cancelled = true;
    };
  }, [query]);

  useEffect(() => {
    const q = query.trim();
    if (q.length < 2 || pasted) {
      setResults([]);
      setSearching(false);
      return;
    }
    let cancelled = false;
    setSearching(true);
    const timer = setTimeout(async () => {
      try {
        const rows = await invoke<ChatRoomRow[]>('search_chat_rooms', { query: q });
        if (!cancelled) setResults((rows ?? []).map(asStream));
      } catch (err) {
        Logger.warn('[AddChat] search failed:', err);
        if (!cancelled) setResults([]);
      } finally {
        if (!cancelled) setSearching(false);
      }
    }, 320);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [query, pasted]);

  const pick = (stream: TwitchStream) => {
    addTab(
      stream.user_login,
      stream.user_id || null,
      stream.user_name || stream.user_login,
      stream.profile_image_url ?? null,
      streamProvider(stream),
    );
    setQuery('');
    onClose();
  };

  const list = pasted ? [pasted] : query.trim().length >= 2 ? results : liveNow;

  return (
    <MobileSheet open={open} onClose={onClose} title="Add a chat" maxHeightFraction={0.7}>
      <div className="flex items-center gap-2 glass-input px-3 mb-2">
        <MagnifyingGlass size={16} className="text-textMuted shrink-0" />
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search, or paste a kick.com link"
          className="flex-1 bg-transparent py-2.5 text-[15px] text-textPrimary placeholder:text-textMuted outline-none"
          autoCapitalize="off"
          autoCorrect="off"
        />
      </div>

      {query.trim().length < 2 && !pasted && (
        <div className="px-1 pb-1.5 text-[12px] font-semibold uppercase tracking-wide text-textMuted">
          Live now
        </div>
      )}

      {searching && list.length === 0 ? (
        <div className="py-6 text-center text-sm text-textMuted">Searching…</div>
      ) : list.length === 0 ? (
        <div className="py-6 text-center text-sm text-textMuted">
          {query.trim().length >= 2
            ? 'No channels found.'
            : 'Nobody you follow is live. Search above to open any channel.'}
        </div>
      ) : (
        <div className="flex flex-col">
          {list.map((stream) => {
            const already = openSet.has(streamKey(stream));
            // Followed streams come from the live query, so absent `is_live`
            // there still means live. Search sets it explicitly.
            const live = stream.is_live ?? true;
            const avatar = stream.profile_image_url;
            return (
              <button
                key={streamKey(stream)}
                onClick={() => !already && pick(stream)}
                disabled={already}
                className="flex items-center gap-3 py-2 px-1 text-left active:opacity-70 disabled:opacity-45"
              >
                <div className="relative shrink-0">
                  {avatar ? (
                    <SevenTvAvatar
                      cosmeticsId={sevenTvIdForStream(stream)}
                      px={36}
                      src={avatar}
                      alt=""
                      className="w-9 h-9 rounded-full object-cover"
                      draggable={false}
                    />
                  ) : (
                    <div className="w-9 h-9 rounded-full bg-surface flex items-center justify-center text-[13px] font-semibold text-textMuted">
                      {(stream.user_name || stream.user_login).charAt(0).toUpperCase()}
                    </div>
                  )}
                  {/* Live state rides the avatar rather than sitting in the
                      metadata column, so it reads at a glance. NOT `.live-dot`:
                      that class is the whole LIVE badge (padded pill, gradient,
                      border, dot via ::before) and renders as an empty pill when
                      used bare. The ring is the row background so the dot stays
                      legible against a busy avatar. */}
                  {live && (
                    <span
                      className="absolute -bottom-0.5 -right-0.5 w-3 h-3 rounded-full border-2"
                      style={{
                        backgroundColor: 'var(--color-live)',
                        borderColor: 'var(--color-background)',
                      }}
                    />
                  )}
                </div>
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-1.5 text-[15px] text-textPrimary">
                    <span className="truncate">{stream.user_name || stream.user_login}</span>
                    {!isTwitchStream(stream) && (
                      <ProviderMark provider={streamProvider(stream)} size={12} />
                    )}
                  </div>
                  <div className="text-[12.5px] text-textMuted truncate">
                    {stream === pasted ? 'Open this Kick chat' : live ? stream.game_name || 'Live' : 'Offline'}
                  </div>
                </div>
                {already ? (
                  <span className="text-[12px] text-textMuted shrink-0">Open</span>
                ) : (
                  live &&
                  stream.viewer_count > 0 && (
                    <span className="flex items-center gap-1 shrink-0 text-[12px] text-textMuted tabular-nums">
                      <UsersThree size={13} weight="fill" />
                      {stream.viewer_count.toLocaleString()}
                    </span>
                  )
                )}
              </button>
            );
          })}
        </div>
      )}
    </MobileSheet>
  );
};
