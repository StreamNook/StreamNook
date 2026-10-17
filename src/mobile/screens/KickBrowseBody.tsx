// Browse, on Kick: the live directory, channel search, and categories.
//
// Its own component rather than branches through BrowseScreen, because every
// Twitch piece there (Helix paging, hype trains, drops, watch streaks) has no
// Kick counterpart, and a Kick id handed to any of them names an unrelated
// Twitch channel. The invoke shapes are the desktop Home's, which already
// drives these same commands.
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { X } from 'phosphor-react';
import { useAppStore } from '../../stores/AppStore';
import { SettleIn, useSettleIn } from '../ui/SettleIn';
import { MobileStreamCard } from '../ui/MobileStreamCard';
import { PullToRefresh } from '../ui/PullToRefresh';
import { SkeletonCards } from '../ui/SkeletonCards';
import { AdaptiveGrid } from '../ui/AdaptiveGrid';
import { Logger } from '../../utils/logger';
import { streamKey } from '../../utils/streamProvider';
import { bumpPreviewStamp } from '../followRefresh';
import type { TwitchStream } from '../../types';
import type { ProviderCategory } from '../../types/providers';
import type { StreamViewMode } from './FollowingScreen';

// Kick's sorted directory endpoint has no cursor and caps at 100.
const DIRECTORY_LIMIT = 100;
const CATEGORY_LIMIT = 40;

// Kick's category list barely moves within a session; pull-to-refresh reloads.
let kickCategoriesCache: ProviderCategory[] | null = null;

/** Every row this screen hands on is a Kick row, even if a response left it unstamped. */
const asKick = (streams: TwitchStream[] | undefined): TwitchStream[] =>
  (streams ?? []).map((s) => ({ ...s, provider: 'kick' as const }));

export const KickBrowseBody: React.FC<{
  mode: 'live' | 'categories';
  query: string;
  view: StreamViewMode;
  category: ProviderCategory | null;
  onPickCategory: (c: ProviderCategory) => void;
  onClearCategory: () => void;
}> = ({ mode, query, view, category, onPickCategory, onClearCategory }) => {
  const startStream = useAppStore((s) => s.startStream);
  const openKick = (row: TwitchStream) => void startStream(row.user_login, { ...row, provider: 'kick' });
  const [streams, setStreams] = useState<TwitchStream[] | null>(null);
  const [categories, setCategories] = useState<ProviderCategory[] | null>(kickCategoriesCache);
  const [loading, setLoading] = useState(false);
  const seq = useRef(0);
  const trimmed = query.trim();

  const loadStreams = useCallback(async () => {
    const mine = ++seq.current;
    setLoading(true);
    try {
      const page = trimmed
        ? await invoke<{ streams: TwitchStream[] }>('provider_search', { provider: 'kick', query: trimmed })
        : await invoke<{ streams: TwitchStream[] }>('provider_directory', {
            provider: 'kick',
            category: category?.id ?? null,
            limit: DIRECTORY_LIMIT,
          });
      if (mine === seq.current) setStreams(asKick(page?.streams));
    } catch (err) {
      Logger.warn('[KickBrowse] streams failed:', err);
      if (mine === seq.current) setStreams([]);
    } finally {
      if (mine === seq.current) setLoading(false);
    }
  }, [trimmed, category?.id]);

  const loadCategories = useCallback(async () => {
    setLoading(true);
    try {
      const page = await invoke<{ categories: ProviderCategory[] }>('provider_categories', {
        provider: 'kick',
        limit: CATEGORY_LIMIT,
      });
      kickCategoriesCache = page?.categories ?? [];
      setCategories(kickCategoriesCache);
    } catch (err) {
      Logger.warn('[KickBrowse] categories failed:', err);
      setCategories((prev) => prev ?? []);
    } finally {
      setLoading(false);
    }
  }, []);

  // Streams: immediately for the directory, debounced while typing a search.
  useEffect(() => {
    if (mode !== 'live') return;
    const t = setTimeout(() => void loadStreams(), trimmed ? 350 : 0);
    return () => clearTimeout(t);
  }, [mode, trimmed, loadStreams]);

  useEffect(() => {
    if (mode === 'categories' && categories === null) void loadCategories();
  }, [mode, categories, loadCategories]);

  const refresh = async () => {
    bumpPreviewStamp();
    if (mode === 'live') await loadStreams();
    else await loadCategories();
  };

  // There is no category search command for Kick, and the list is one page, so
  // searching filters what is already here.
  const shownCategories = (categories ?? []).filter(
    (c) => !trimmed || c.name.toLowerCase().includes(trimmed.toLowerCase()),
  );
  // A search can find a channel that is not streaming (Rust settles it from
  // Kick's own live answer): it goes in the offline list, never a live card.
  const shownStreams = (streams ?? []).filter((s) => s.is_live !== false);
  const offlineStreams = (streams ?? []).filter((s) => s.is_live === false);

  const streamsSettled = useSettleIn(
    !loading && shownStreams.length > 0,
    `${view}:${trimmed ? `q:${trimmed}` : `dir:${category?.id ?? ''}`}`,
  );
  const categoriesSettled = useSettleIn(!loading && shownCategories.length > 0, 'kick-top');

  return (
    <PullToRefresh onRefresh={refresh}>
      {mode === 'live' ? (
        <>
          {category && !trimmed && (
            <div className="px-4 pb-2.5">
              <span className="glass-badge inline-flex items-center gap-1.5 pl-3 pr-1 py-1 rounded-full text-[13px] text-textPrimary">
                {category.name}
                <button
                  onClick={onClearCategory}
                  className="flex items-center justify-center w-6 h-6 text-textMuted"
                  aria-label={`Show all of Kick, not just ${category.name}`}
                >
                  <X size={13} weight="bold" />
                </button>
              </span>
            </div>
          )}
          {streams === null || (loading && shownStreams.length === 0 && offlineStreams.length === 0) ? (
            <SkeletonCards />
          ) : shownStreams.length === 0 && offlineStreams.length > 0 ? (
            <OfflineChannels channels={offlineStreams} onOpen={openKick} />
          ) : shownStreams.length === 0 ? (
            <div className="flex flex-col items-center justify-center py-20 gap-1">
              <div className="text-sm text-textMuted">
                {trimmed ? 'No Kick channels found.' : 'Nothing live on Kick right now.'}
              </div>
              <div className="text-[13px] text-textMuted">Pull down to refresh.</div>
            </div>
          ) : (
            <AdaptiveGrid
              variant={view === 'list' ? 'row' : 'card'}
              gap={view === 'list' ? 8 : 12}
              className="px-4 sn-tabbar-clearance"
            >
              {shownStreams.map((s, i) => (
                <SettleIn key={streamKey(s)} index={i} settled={streamsSettled}>
                  <MobileStreamCard
                    stream={s}
                    onPress={openKick}
                    variant={view === 'list' ? 'row' : 'card'}
                    showPlatform={false}
                  />
                </SettleIn>
              ))}
            </AdaptiveGrid>
          )}
          {shownStreams.length > 0 && offlineStreams.length > 0 && (
            <OfflineChannels channels={offlineStreams} onOpen={openKick} />
          )}
        </>
      ) : categories === null || (loading && shownCategories.length === 0) ? (
        <SkeletonCards />
      ) : shownCategories.length === 0 ? (
        <div className="flex items-center justify-center py-20 text-sm text-textMuted">
          {trimmed ? 'No categories found.' : 'No categories yet, pull to refresh.'}
        </div>
      ) : (
        <div className="grid grid-cols-3 sm:grid-cols-4 md:grid-cols-5 lg:grid-cols-6 gap-3 px-4 sn-tabbar-clearance">
          {shownCategories.map((c, i) => (
            <SettleIn key={c.id} index={i} settled={categoriesSettled}>
              <button
                onClick={() => onPickCategory(c)}
                className="glass-panel media-card p-2 text-left active:opacity-80 transition-opacity w-full"
              >
                {c.thumbnail ? (
                  <img
                    loading="lazy"
                    decoding="async"
                    src={c.thumbnail}
                    alt=""
                    className="w-full aspect-[3/4] object-cover rounded mb-1.5"
                    draggable={false}
                  />
                ) : (
                  <div className="w-full aspect-[3/4] rounded mb-1.5 bg-surface" />
                )}
                <div className="text-[13px] font-medium text-textPrimary line-clamp-1">{c.name}</div>
              </button>
            </SettleIn>
          ))}
        </div>
      )}
    </PullToRefresh>
  );
};

/** Channels a search found that are not streaming: a plain list (no preview,
 *  no viewers, nothing to play), tapped to open the channel. */
const OfflineChannels: React.FC<{ channels: TwitchStream[]; onOpen: (row: TwitchStream) => void }> = ({
  channels,
  onOpen,
}) => (
  <div className="px-4 pt-1 sn-tabbar-clearance">
    <div className="px-1 pb-2 text-[12px] font-semibold uppercase tracking-wide text-textMuted">Offline channels</div>
    <div className="glass-panel media-card overflow-hidden">
      {channels.map((c, i) => (
        <button
          key={streamKey(c)}
          onClick={() => onOpen(c)}
          className={`flex w-full items-center gap-3 px-3.5 py-2.5 text-left active:opacity-70 ${i > 0 ? 'border-t border-borderSubtle' : ''}`}
        >
          {c.profile_image_url ? (
            <img src={c.profile_image_url} alt="" className="h-9 w-9 shrink-0 rounded-full object-cover" draggable={false} />
          ) : (
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-surface text-[14px] font-semibold text-textSecondary">
              {(c.user_name || c.user_login).charAt(0).toUpperCase()}
            </div>
          )}
          <div className="min-w-0">
            <div className="truncate text-[15px] font-semibold text-textPrimary">{c.user_name || c.user_login}</div>
            <div className="truncate text-[12.5px] text-textSecondary">{c.game_name || 'Offline'}</div>
          </div>
        </button>
      ))}
    </div>
  </div>
);
