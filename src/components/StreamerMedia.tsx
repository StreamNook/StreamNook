import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Heart, Loader2, Search, X } from 'lucide-react';

import { useAppStore, clipSourceOf } from '../stores/AppStore';
import type { TwitchClip, TwitchVideo } from '../types';
import { GlassSelect } from './ui/GlassSelect';
import { MediaCard } from './MediaCard';
import { formatCardDate, mediaKindOfVideo, videoDurationLabel, vodThumbUrl, VOD_FALLBACK_THUMB } from '../utils/vodProgress';
import { Logger } from '../utils/logger';


// One streamer's Clips or Videos, rendered like the category browser's grid.
// Clicking a clip opens the centered clip modal (you stay in the browser);
// clicking a VOD closes the profile and plays it in the main player.
export default function StreamerMedia({
  broadcasterId,
  kind,
  channelAvatarUrl,
  channelPartner,
}: {
  broadcasterId: string;
  kind: 'clips' | 'videos';
  /** The profile already knows the channel's avatar; the cards reuse it. */
  channelAvatarUrl?: string;
  channelPartner?: boolean;
}) {
  const openClipModal = useAppStore((s) => s.openClipModal);
  const playMedia = useAppStore((s) => s.playMedia);
  const setProfileModalUser = useAppStore((s) => s.setProfileModalUser);

  const [clips, setClips] = useState<TwitchClip[]>([]);
  const [videos, setVideos] = useState<TwitchVideo[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  // Default to all-time so a streamer's clips always surface (their best ones
  // are often older); the filter lets you narrow to recent.
  const [period, setPeriod] = useState('all'); // clips only
  // Videos have no time-window filter on Twitch (Helix can't do it for a user,
  // and the website doesn't offer one); instead they get Sort (Recent/Popular)
  // + broadcast Type, mirroring twitch.tv's Videos tab.
  const [videoSort, setVideoSort] = useState('time'); // 'time' | 'views', videos only
  const [videoType, setVideoType] = useState(''); // '' = all | archive | highlight | upload
  const [query, setQuery] = useState('');
  const [loading, setLoading] = useState(true);
  const [loadingMore, setLoadingMore] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  // game_id → category name, resolved lazily for clips (clips carry game_id, not
  // the name). The ref tracks which ids we've already requested to avoid refetch.
  const [gameNames, setGameNames] = useState<Record<string, string>>({});
  const requestedGamesRef = useRef<Set<string>>(new Set());

  // clip slug → reaction ("likes") counts. Reactions aren't in the clip list
  // query — Twitch loads them per-clip on the clip page — so we batch-fetch them
  // for the loaded grid and fill them in after. The ref dedupes requests.
  const [reactions, setReactions] = useState<Record<string, { total: number; like: number }>>({});
  const requestedReactionsRef = useRef<Set<string>>(new Set());
  // Clips come view-sorted from Twitch; "likes" sorts the loaded grid client-side
  // (reactions arrive after the list, and Twitch won't sort the list by them).
  const [clipSort, setClipSort] = useState<'views' | 'likes'>('views');

  // Initial load (reloads when the clip period changes).
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      setLoading(true);
      setErrorMsg(null);
      try {
        if (kind === 'clips') {
          const [items, next] = await invoke<[TwitchClip[], string | null]>('get_clips_by_broadcaster', {
            broadcasterId,
            limit: 40,
            cursor: null,
            period,
          });
          if (cancelled) return;
          setClips(items);
          setCursor(next);
        } else {
          const [items, next] = await invoke<[TwitchVideo[], string | null]>('get_user_videos', {
            userId: broadcasterId,
            sort: videoSort,
            videoType: videoType || null,
            limit: 40,
            cursor: null,
          });
          if (cancelled) return;
          setVideos(items);
          setCursor(next);
        }
      } catch (e) {
        if (cancelled) return;
        Logger.error('[StreamerMedia] load failed:', e);
        setErrorMsg(String(e).replace(/^Error:\s*/, '') || 'unknown error');
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [broadcasterId, kind, period, videoSort, videoType]);

  // Resolve clip categories → names. The GQL clip path returns game_name inline,
  // so seed from that first and only fall back to a batched lookup for any clip
  // still missing one (deduped via the ref).
  useEffect(() => {
    if (kind !== 'clips') return;
    const inline: Record<string, string> = {};
    for (const c of clips) {
      if (c.game_id && c.game_name && !requestedGamesRef.current.has(c.game_id)) {
        inline[c.game_id] = c.game_name;
        requestedGamesRef.current.add(c.game_id);
      }
    }
    if (Object.keys(inline).length > 0) {
      setGameNames((prev) => ({ ...prev, ...inline }));
    }
    const missing = [
      ...new Set(
        clips
          .map((c) => c.game_id)
          .filter((id): id is string => !!id && !requestedGamesRef.current.has(id)),
      ),
    ];
    if (missing.length === 0) return;
    missing.forEach((id) => requestedGamesRef.current.add(id));
    let cancelled = false;
    invoke<Array<{ id?: string; name?: string }>>('get_games_by_ids', { ids: missing })
      .then((games) => {
        if (cancelled) return;
        setGameNames((prev) => {
          const next = { ...prev };
          for (const g of games) if (g?.id) next[g.id] = g.name || '';
          return next;
        });
      })
      .catch((e) => Logger.warn('[StreamerMedia] category resolve failed:', e));
    return () => {
      cancelled = true;
    };
  }, [clips, kind]);

  // Batch-fetch reactions ("likes") for any newly loaded clips (deduped via the
  // ref). Fails soft: if the user isn't logged in the call errors and the grid
  // just shows no reaction counts. clip.id is the slug = the reaction contentKey.
  useEffect(() => {
    if (kind !== 'clips') return;
    const missing = clips
      .map((c) => c.id)
      .filter((id) => !!id && !requestedReactionsRef.current.has(id));
    if (missing.length === 0) return;
    missing.forEach((id) => requestedReactionsRef.current.add(id));
    let cancelled = false;
    invoke<Array<{ id: string; total: number; like: number }>>('get_clip_reactions', { slugs: missing })
      .then((rows) => {
        if (cancelled) return;
        setReactions((prev) => {
          const next = { ...prev };
          for (const r of rows) next[r.id] = { total: r.total, like: r.like };
          return next;
        });
      })
      // Fails soft: if the fetch errors (e.g. not authed for drops) the grid just
      // shows no reaction counts — no toast, since it's an optional enhancement.
      .catch((e) => Logger.warn('[StreamerMedia] reactions fetch failed:', e));
    return () => {
      cancelled = true;
    };
  }, [clips, kind]);

  const loadMore = async () => {
    if (!cursor || loadingMore) return;
    setLoadingMore(true);
    try {
      if (kind === 'clips') {
        const [items, next] = await invoke<[TwitchClip[], string | null]>('get_clips_by_broadcaster', {
          broadcasterId,
          limit: 40,
          cursor,
          period,
        });
        // A top-viewed clip near a page boundary can come back on the next page;
        // drop already-present slugs to avoid duplicate React keys and cards.
        setClips((prev) => {
          const seen = new Set(prev.map((c) => c.id));
          return [...prev, ...items.filter((c) => !seen.has(c.id))];
        });
        setCursor(next);
      } else {
        const [items, next] = await invoke<[TwitchVideo[], string | null]>('get_user_videos', {
          userId: broadcasterId,
          sort: videoSort,
          videoType: videoType || null,
          limit: 40,
          cursor,
        });
        // Same page-boundary overlap guard as clips: drop already-present ids.
        setVideos((prev) => {
          const seen = new Set(prev.map((v) => v.id));
          return [...prev, ...items.filter((v) => !seen.has(v.id))];
        });
        setCursor(next);
      }
    } catch (e) {
      Logger.error('[StreamerMedia] load more failed:', e);
    } finally {
      setLoadingMore(false);
    }
  };

  const q = query.trim().toLowerCase();
  const shownClips = q
    ? clips.filter(
        (c) =>
          c.title.toLowerCase().includes(q) ||
          c.creator_name.toLowerCase().includes(q) ||
          (gameNames[c.game_id] || '').toLowerCase().includes(q),
      )
    : clips;
  const shownVideos = q
    ? videos.filter((v) => v.title.toLowerCase().includes(q) || v.user_name.toLowerCase().includes(q))
    : videos;

  // Clips arrive view-sorted from Twitch; "Most reactions" reorders the loaded
  // grid by reaction total (clips whose reactions haven't arrived yet sink to the
  // bottom until they load). "Most viewed" keeps Twitch's original order.
  const displayedClips =
    clipSort === 'likes'
      ? [...shownClips].sort((a, b) => (reactions[b.id]?.total ?? -1) - (reactions[a.id]?.total ?? -1))
      : shownClips;

  const renderClip = (clip: TwitchClip) => {
    const category = gameNames[clip.game_id];
    const reaction = reactions[clip.id];
    return (
      <MediaCard
        key={clip.id}
        kind="clip"
        title={clip.title}
        thumbnailUrl={clip.thumbnail_url || VOD_FALLBACK_THUMB}
        durationLabel={`${clip.duration.toFixed(1)}s`}
        viewCount={clip.view_count}
        topRight={
          reaction && (
            <div className="glass-badge absolute right-2 top-2 flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] font-medium text-white">
              <Heart size={10} />
              {reaction.total.toLocaleString()}
            </div>
          )
        }
        channel={{ name: clip.broadcaster_name, avatarUrl: channelAvatarUrl, partner: channelPartner }}
        trailing={formatCardDate(clip.created_at)}
        category={category}
        note={`Clipped by ${clip.creator_name}`}
        onClick={() => openClipModal(clip.url, { ...clip, clip_source: clipSourceOf(clip) })}
      />
    );
  };

  const renderVideo = (video: TwitchVideo) => (
    <MediaCard
      key={video.id}
      kind={mediaKindOfVideo(video.type)}
      title={video.title}
      thumbnailUrl={vodThumbUrl(video.thumbnail_url)}
      durationLabel={videoDurationLabel(video)}
      viewCount={video.view_count}
      progress={video.progress}
      lengthSeconds={video.length_seconds}
      status={video.status}
      channel={{ name: video.user_name, avatarUrl: channelAvatarUrl, partner: channelPartner }}
      trailing={formatCardDate(video.created_at)}
      category={video.game_name}
      onClick={() => {
        setProfileModalUser(null);
        playMedia('video', video.url, video);
      }}
    />
  );

  const loadedCount = kind === 'clips' ? clips.length : videos.length;
  const shownCount = kind === 'clips' ? shownClips.length : shownVideos.length;

  return (
    <div className="scrollbar-thin h-full overflow-y-auto p-4">
      {/* Toolbar: search (titles + creators) + clip period filter */}
      <div className="mb-3 flex items-center justify-between gap-2">
        <div className="group relative">
          <div className="pointer-events-none absolute inset-y-0 left-0 flex items-center pl-3 text-textSecondary transition-colors group-focus-within:text-accent">
            <Search size={14} />
          </div>
          <input
            type="text"
            placeholder={`Search ${kind}...`}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            className="glass-input w-[160px] !rounded-lg py-1.5 pl-9 pr-8 text-sm font-medium text-textPrimary outline-none transition-all placeholder:text-textSecondary/50 focus:w-[240px]"
          />
          {query && (
            <button
              onClick={() => setQuery('')}
              className="absolute inset-y-0 right-0 flex items-center pr-3 text-textSecondary transition-colors hover:text-accent"
            >
              <X size={14} />
            </button>
          )}
        </div>
        {kind === 'clips' ? (
          <div className="flex items-center gap-2">
            <GlassSelect
              value={clipSort}
              onChange={(val) => setClipSort(val as 'views' | 'likes')}
              options={[
                { value: 'views', label: 'Most Viewed' },
                { value: 'likes', label: 'Most Reactions' },
              ]}
            />
            <GlassSelect
              value={period}
              onChange={(val) => setPeriod(val)}
              options={[
                { value: '24h', label: 'Last 24 Hours' },
                { value: '7d', label: 'Last 7 Days' },
                { value: '30d', label: 'Last 30 Days' },
                { value: 'all', label: 'All Time' },
              ]}
            />
          </div>
        ) : (
          <div className="flex items-center gap-2">
            <GlassSelect
              value={videoSort}
              onChange={(val) => setVideoSort(val)}
              options={[
                { value: 'time', label: 'Recent' },
                { value: 'views', label: 'Popular' },
              ]}
            />
            <GlassSelect
              value={videoType}
              onChange={(val) => setVideoType(val)}
              options={[
                { value: '', label: 'All Videos' },
                { value: 'archive', label: 'Past Broadcasts' },
                { value: 'highlight', label: 'Highlights' },
                { value: 'upload', label: 'Uploads' },
              ]}
            />
          </div>
        )}
      </div>

      {loading ? (
        <div className="flex h-[300px] items-center justify-center">
          <Loader2 className="h-8 w-8 animate-spin text-accent" />
        </div>
      ) : errorMsg ? (
        <div className="flex h-[300px] items-center justify-center">
          <div className="glass-panel max-w-md p-6 text-center">
            <h3 className="mb-1 text-base font-bold text-textPrimary">Couldn't load {kind}</h3>
            <p className="break-words text-sm text-textSecondary">{errorMsg}</p>
          </div>
        </div>
      ) : loadedCount === 0 ? (
        <div className="flex h-[300px] items-center justify-center">
          <div className="glass-panel max-w-sm p-6 text-center">
            <h3 className="mb-1 text-base font-bold text-textPrimary">
              No {kind === 'clips' ? 'clips' : 'videos'} yet
            </h3>
            <p className="text-sm text-textSecondary">
              {kind === 'clips'
                ? 'This channel has no clips for the selected period.'
                : 'This channel has no past broadcasts available.'}
            </p>
          </div>
        </div>
      ) : shownCount === 0 ? (
        <div className="flex h-[300px] items-center justify-center">
          <div className="glass-panel max-w-sm p-6 text-center">
            <h3 className="mb-1 text-base font-bold text-textPrimary">No results</h3>
            <p className="text-sm text-textSecondary">
              No {kind} match "{query}".
            </p>
          </div>
        </div>
      ) : (
        <>
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
            {kind === 'clips' ? displayedClips.map(renderClip) : shownVideos.map(renderVideo)}
          </div>
          {cursor && !q && (
            <div className="flex justify-center py-5">
              <button
                onClick={loadMore}
                disabled={loadingMore}
                className="glass-button rounded-lg px-5 py-2 text-sm font-semibold text-textSecondary transition-colors hover:text-textPrimary disabled:opacity-50"
              >
                {loadingMore ? <Loader2 className="h-4 w-4 animate-spin" /> : 'Load more'}
              </button>
            </div>
          )}
        </>
      )}
    </div>
  );
}
