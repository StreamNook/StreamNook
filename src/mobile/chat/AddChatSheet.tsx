// Pick another channel's chat to open in a tab.
//
// Live follows come first because that is where moderating usually happens, and
// they need no network call. Search covers everyone else, and deliberately
// includes offline channels: chat is joinable whether or not the channel is
// streaming, which is exactly the case for moderating a friend's quiet room.
// `search_channels` already passes `live_only=false` and returns `is_live` plus
// `profile_image_url`, so offline results need no extra call.
import React, { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Eye, MagnifyingGlass } from 'phosphor-react';
import { MobileSheet } from '../ui/MobileSheet';
import { useAppStore } from '../../stores/AppStore';
import { useChatTabsStore } from './chatTabsStore';
import { Logger } from '../../utils/logger';
import type { TwitchStream } from '../../types';

export const AddChatSheet: React.FC<{ open: boolean; onClose: () => void }> = ({
  open,
  onClose,
}) => {
  const followedStreams = useAppStore((s) => s.followedStreams);
  const openTabs = useChatTabsStore((s) => s.tabs);
  const addTab = useChatTabsStore((s) => s.addTab);

  const [query, setQuery] = useState('');
  const [results, setResults] = useState<TwitchStream[]>([]);
  const [searching, setSearching] = useState(false);

  const openSet = useMemo(
    () => new Set(openTabs.map((t) => t.channel)),
    [openTabs],
  );

  useEffect(() => {
    const q = query.trim();
    if (q.length < 2) {
      setResults([]);
      return;
    }
    let cancelled = false;
    setSearching(true);
    const timer = setTimeout(async () => {
      try {
        const found = await invoke<TwitchStream[]>('search_channels', { query: q });
        if (!cancelled) setResults(found ?? []);
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
  }, [query]);

  const pick = (stream: TwitchStream) => {
    addTab(
      stream.user_login,
      stream.user_id || null,
      stream.user_name || stream.user_login,
      stream.profile_image_url ?? null,
    );
    setQuery('');
    onClose();
  };

  const list = query.trim().length >= 2 ? results : followedStreams;

  return (
    <MobileSheet open={open} onClose={onClose} title="Add a chat" maxHeightFraction={0.7}>
      <div className="flex items-center gap-2 glass-input px-3 mb-2">
        <MagnifyingGlass size={16} className="text-textMuted shrink-0" />
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search any channel, live or not"
          className="flex-1 bg-transparent py-2.5 text-[15px] text-textPrimary placeholder:text-textMuted outline-none"
          autoCapitalize="off"
          autoCorrect="off"
        />
      </div>

      {query.trim().length < 2 && (
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
            const already = openSet.has(stream.user_login.toLowerCase());
            // Followed streams come from the live query, so absent `is_live`
            // there still means live. Search sets it explicitly.
            const live = stream.is_live ?? true;
            const avatar = stream.profile_image_url;
            return (
              <button
                key={stream.user_login}
                onClick={() => !already && pick(stream)}
                disabled={already}
                className="flex items-center gap-3 py-2 px-1 text-left active:opacity-70 disabled:opacity-45"
              >
                <div className="relative shrink-0">
                  {avatar ? (
                    <img
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
                  <div className="text-[15px] text-textPrimary truncate">
                    {stream.user_name || stream.user_login}
                  </div>
                  <div className="text-[12.5px] text-textMuted truncate">
                    {live ? stream.game_name || 'Live' : 'Offline'}
                  </div>
                </div>
                {already ? (
                  <span className="text-[12px] text-textMuted shrink-0">Open</span>
                ) : (
                  live &&
                  stream.viewer_count > 0 && (
                    <span className="flex items-center gap-1 shrink-0 text-[12px] text-textMuted tabular-nums">
                      <Eye size={13} />
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
