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
import { SkeletonCards, SkeletonTiles } from '../ui/SkeletonCards';
import { CategoryTile } from '../ui/CategoryTile';
import { TILE_GRID_CLASS } from '../ui/categoryTileLayout';
import { AdaptiveGrid } from '../ui/AdaptiveGrid';
import { Logger } from '../../utils/logger';
import { streamKey } from '../../utils/streamProvider';
import { bumpPreviewStamp } from '../followRefresh';
import { EmptyNote } from '../ui/EmptyNote';
import { OfflineChannelsGroup } from './OfflineChannelsGroup';
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
            <OfflineChannelsGroup className="pt-1 sn-tabbar-clearance" channels={offlineStreams} onOpen={openKick} />
          ) : shownStreams.length === 0 ? (
            <EmptyNote text={trimmed ? 'No Kick channels found.' : 'Nothing live on Kick right now.'} />
          ) : (
            // The clearance wraps both groups, as on Twitch, so offline
            // channels follow the live cards instead of a tab bar's height
            // further down.
            <div className="sn-tabbar-clearance">
              <AdaptiveGrid variant={view === 'list' ? 'row' : 'card'} gap={view === 'list' ? 8 : 12} className="px-4">
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
              {offlineStreams.length > 0 && (
                <OfflineChannelsGroup className="mt-5" channels={offlineStreams} onOpen={openKick} />
              )}
            </div>
          )}
        </>
      ) : categories === null || (loading && shownCategories.length === 0) ? (
        <SkeletonTiles />
      ) : shownCategories.length === 0 ? (
        <EmptyNote text={trimmed ? 'No categories found.' : 'No categories yet, pull to refresh.'} hint={null} />
      ) : (
        <div className={`${TILE_GRID_CLASS} px-4 sn-tabbar-clearance`}>
          {shownCategories.map((c, i) => (
            <SettleIn key={c.id} index={i} settled={categoriesSettled}>
              <CategoryTile
                name={c.name}
                art={c.thumbnail}
                viewers={c.viewer_count}
                viewersText={c.viewers_line}
                twoLineName={!!trimmed}
                onPress={() => onPickCategory(c)}
              />
            </SettleIn>
          ))}
        </div>
      )}
    </PullToRefresh>
  );
};
