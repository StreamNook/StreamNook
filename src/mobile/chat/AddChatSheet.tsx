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
import { ProviderMark } from '../../components/ProviderLogo';
import { useFollowsStore } from '../../stores/followsStore';
import { isTwitchStream, streamKey, streamProvider } from '../../utils/streamProvider';
import { mergeFollowedLive } from '../followingMerge';
import { orderSearchResults } from './searchOrder';

/** A Kick slug from a pasted kick.com link or a `kick:slug` / `kick/slug` prefix. */
function parseKickLink(input: string): string | null {
  const s = input.trim();
  const m =
    s.match(/^(?:https?:\/\/)?(?:www\.)?kick\.com\/(@?[a-z0-9_-]+)/i) ||
    s.match(/^kick[:/](@?[a-z0-9_-]+)$/i);
  return m ? m[1].replace(/^@/, '').toLowerCase() : null;
}

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

  // Live Kick follows sit beside the Twitch ones.
  const providerLive = useFollowsStore((s) => s.liveByKey);
  const liveNow = useMemo(() => mergeFollowedLive(followedStreams, providerLive), [followedStreams, providerLive]);
  // A pasted kick.com link (or `kick:slug`) opens that room directly, live or
  // not, without a search round trip.
  const kickLink = parseKickLink(query);
  const pasted = useMemo<TwitchStream | null>(
    () =>
      kickLink
        ? ({
            id: `kick:${kickLink}`,
            user_id: '',
            user_login: kickLink,
            user_name: kickLink,
            title: '',
            viewer_count: 0,
            game_name: '',
            thumbnail_url: '',
            started_at: '',
            provider: 'kick',
            is_live: false,
          } as TwitchStream)
        : null,
    [kickLink],
  );

  const openSet = useMemo(
    () => new Set(openTabs.map((t) => t.channel)),
    [openTabs],
  );

  useEffect(() => {
    const q = query.trim();
    if (q.length < 2 || parseKickLink(q)) {
      setResults([]);
      return;
    }
    let cancelled = false;
    setSearching(true);
    const timer = setTimeout(async () => {
      try {
        // Both platforms, settled independently so one failing never empties
        // the other.
        const [tw, kick] = await Promise.allSettled([
          invoke<TwitchStream[]>('search_channels', { query: q }),
          invoke<{ streams: TwitchStream[] }>('provider_search', { provider: 'kick', query: q }),
        ]);
        if (tw.status === 'rejected') Logger.warn('[AddChat] twitch search failed:', tw.reason);
        if (kick.status === 'rejected') Logger.warn('[AddChat] kick search failed:', kick.reason);
        if (!cancelled) {
          setResults(
            orderSearchResults([
              ...(tw.status === 'fulfilled' ? (tw.value ?? []) : []),
              ...(kick.status === 'fulfilled'
                ? (kick.value?.streams ?? []).map((st) => ({ ...st, provider: 'kick' as const }))
                : []),
            ]),
          );
        }
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
