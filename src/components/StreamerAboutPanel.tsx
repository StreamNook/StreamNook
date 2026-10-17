import React, { useEffect, useState, memo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ExternalLink, Users } from 'lucide-react';
import { ChannelAboutData, ChannelPanel as ChannelPanelType } from '../types/panels';
import { Logger } from '../utils/logger';
import { useAppStore } from '../stores/AppStore';
import { formatViewerCount } from '../utils/streamStats';
import { SevenTvAvatar } from './SevenTvAvatar';
import { SocialLinks } from './SocialLinks';
import { sevenTvIdForStream } from '../services/sevenTvPictures';

/** One live channel from Twitch's "Viewers of X also watch" set. Mirrors the
 *  Rust SimilarChannel struct. */
interface SimilarChannel {
  user_id: string;
  user_login: string;
  display_name: string;
  profile_image_url?: string;
  viewer_count: number;
  game_name?: string;
  thumbnail_url?: string;
}

// Session-lifetime cache: About drawers reopen constantly and the set changes
// slowly, so one fetch per channel per session is plenty.
const similarCache = new Map<string, SimilarChannel[]>();

// ============================================================================
// Subcomponents
// ============================================================================

// A link inside a description opens in the browser and wins over the card's
// own link, which would otherwise swallow the click.
const openDescriptionLink = (e: React.MouseEvent) => {
  const anchor = (e.target as HTMLElement).closest('a');
  if (!anchor?.href) return;
  e.preventDefault();
  e.stopPropagation();
  invoke('open_browser_url', { url: anchor.href });
};

const PanelCard = memo(({ panel }: { panel: ChannelPanelType }) => {
  const hasImage = !!panel.image_url;
  const hasLink = !!panel.link_url;
  const hasDescription = !!panel.description;
  const hasTitle = !!panel.title;

  // Extension panels — can't run outside Twitch, skip
  if (panel.panel_type === 'EXTENSION') return null;

  // Image-only panels (no title, no description — just clickable image)
  if (hasImage && !hasTitle && !hasDescription) {
    const img = (
      <img
        src={panel.image_url!}
        alt="Panel"
        className="w-full h-auto object-cover rounded-lg"
        loading="lazy"
        onError={(e) => { (e.currentTarget as HTMLImageElement).style.display = 'none'; }}
      />
    );

    if (hasLink) {
      return (
        <a
          href={panel.link_url!}
          // No target="_blank": the webview opens _blank links externally on its
          // own, which would stack a second tab on top of the invoke() below.
          onClick={(e) => {
            e.preventDefault();
            invoke('open_browser_url', { url: panel.link_url });
          }}
          className="block relative group rounded-lg overflow-hidden mb-3 break-inside-avoid"
        >
          {img}
          <div className="absolute inset-0 bg-black/0 group-hover:bg-black/20 transition-colors flex items-center justify-center">
            <ExternalLink size={18} className="text-white opacity-0 group-hover:opacity-80 transition-opacity drop-shadow-lg" />
          </div>
        </a>
      );
    }
    return <div className="rounded-lg overflow-hidden mb-3 break-inside-avoid">{img}</div>;
  }

  const Wrapper = hasLink ? 'a' : 'div';
  const wrapperProps = hasLink ? {
    href: panel.link_url!,
    // No target="_blank": the webview opens _blank links externally on its
    // own, which would stack a second tab on top of the invoke() below.
    onClick: (e: React.MouseEvent) => {
      e.preventDefault();
      invoke('open_browser_url', { url: panel.link_url });
    },
  } : {};

  return (
    <Wrapper
      {...wrapperProps}
      className={`glass-panel rounded-lg overflow-hidden border border-borderSubtle/30 transition-all duration-200 block mb-3 break-inside-avoid ${
        hasLink ? 'cursor-pointer hover:border-accent/40 hover:bg-white/[0.02] group' : ''
      }`}
    >
      {/* Panel Image */}
      {hasImage && (
        <div className="relative w-full">
          <img
            src={panel.image_url!}
            alt={panel.title || 'Panel image'}
            className="w-full h-auto object-cover"
            loading="lazy"
            onError={(e) => { (e.currentTarget as HTMLImageElement).style.display = 'none'; }}
          />
          {hasLink && (
            <div className="absolute inset-0 bg-black/0 group-hover:bg-black/20 transition-colors flex items-center justify-center">
              <ExternalLink size={18} className="text-white opacity-0 group-hover:opacity-80 transition-opacity drop-shadow-lg" />
            </div>
          )}
        </div>
      )}

      {/* Panel Content (title + description) */}
      {(hasTitle || hasDescription) && (
        <div className="px-3 py-2.5 space-y-1">
          {hasTitle && (
            <div className="flex items-center gap-1.5">
              <h4 className="text-[13px] font-semibold text-textPrimary leading-tight">
                {panel.title}
              </h4>
              {hasLink && !hasImage && (
                <ExternalLink size={10} className="text-accent/50 flex-shrink-0 group-hover:text-accent transition-colors" />
              )}
            </div>
          )}
          {hasDescription && (
            <div
              className="sn-panel-md text-xs text-textSecondary/80 leading-relaxed"
              onClick={openDescriptionLink}
              dangerouslySetInnerHTML={{ __html: panel.description! }}
            />
          )}
        </div>
      )}
    </Wrapper>
  );
});

// Loading skeleton
const LoadingSkeleton = () => (
  <div className="p-4 space-y-4 animate-pulse">
    <div className="flex items-center gap-3">
      <div className="w-14 h-14 rounded-full bg-glass/50" />
      <div className="space-y-2 flex-1">
        <div className="h-4 w-32 bg-glass/50 rounded" />
        <div className="h-3 w-20 bg-glass/30 rounded" />
      </div>
    </div>
    <div className="space-y-1.5">
      <div className="h-3 w-full bg-glass/30 rounded" />
      <div className="h-3 w-4/5 bg-glass/30 rounded" />
    </div>
    <div className="flex gap-2">
      {[1, 2, 3].map((i) => (
        <div key={i} className="h-7 w-20 bg-glass/30 rounded-full" />
      ))}
    </div>
    {[1, 2].map((i) => (
      <div key={i} className="h-32 bg-glass/20 rounded-lg" />
    ))}
  </div>
);

// Empty state
const EmptyState = () => (
  <div className="flex-1 flex flex-col items-center justify-center p-8 text-center gap-3 min-h-[50vh]">
    <div className="w-12 h-12 rounded-full bg-glass/30 flex items-center justify-center">
      <Users size={20} className="text-textSecondary/60" />
    </div>
    <p className="text-sm text-textSecondary">No about info available</p>
    <p className="text-xs text-textSecondary/60">This streamer hasn't set up their panels yet.</p>
  </div>
);

// ============================================================================
// Main Component
// ============================================================================

interface StreamerAboutPanelProps {
  channelLogin: string;
  hideHero?: boolean;
  /** Forwarded to the panel's own scroll container. Lets a host (e.g. the player
   *  About reveal) own the single scroller and read its scroll position directly,
   *  instead of nesting a second overflow that the host can't see. */
  scrollRef?: React.Ref<HTMLDivElement>;
  /** Called after a "Viewers also watch" card starts a new stream, so a modal
   *  host can close itself. The player About reveal doesn't need it: it already
   *  auto-closes when the channel changes. */
  onChannelSwitch?: () => void;
}

const StreamerAboutPanel = memo(({ channelLogin, hideHero, scrollRef, onChannelSwitch }: StreamerAboutPanelProps) => {
  const [aboutData, setAboutData] = useState<ChannelAboutData | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [similarStreams, setSimilarStreams] = useState<SimilarChannel[] | null>(
    () => similarCache.get(channelLogin.toLowerCase()) ?? null
  );

  useEffect(() => {
    let cancelled = false;

    const fetchData = async () => {
      setIsLoading(true);
      setError(null);

      try {
        const data = await invoke<ChannelAboutData>('get_channel_about_data', {
          channelLogin: channelLogin,
        });

        if (!cancelled) {
          setAboutData(data);
          Logger.debug('[StreamerAboutPanel] Loaded about data:', data.panels.length, 'panels,', data.social_links.length, 'social links');
        }
      } catch (err) {
        if (!cancelled) {
          Logger.error('[StreamerAboutPanel] Failed to fetch about data:', err);
          setError(String(err));
        }
      } finally {
        if (!cancelled) {
          setIsLoading(false);
        }
      }
    };

    fetchData();
    return () => { cancelled = true; };
  }, [channelLogin]);

  // "Viewers also watch": independent of the about fetch so a failure here can
  // never affect the rest of the panel. Best-effort; empty result hides the row.
  useEffect(() => {
    const key = channelLogin.toLowerCase();
    const cached = similarCache.get(key);
    if (cached) {
      setSimilarStreams(cached);
      return;
    }
    let cancelled = false;
    setSimilarStreams(null);
    invoke<SimilarChannel[]>('get_similar_channels', { channelLogin: key })
      .then((rows) => {
        const filtered = rows.filter((r) => r.user_login.toLowerCase() !== key);
        similarCache.set(key, filtered);
        if (!cancelled) setSimilarStreams(filtered);
      })
      .catch((err) => {
        Logger.debug('[StreamerAboutPanel] Similar channels unavailable:', err);
        if (!cancelled) setSimilarStreams([]);
      });
    return () => { cancelled = true; };
  }, [channelLogin]);

  const handleSimilarClick = (stream: SimilarChannel) => {
    void useAppStore.getState().startStream(stream.user_login);
    onChannelSwitch?.();
  };

  // Format follower count
  const formatFollowers = (count: number): string => {
    if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M`;
    if (count >= 1_000) return `${(count / 1_000).toFixed(1)}K`;
    return count.toLocaleString();
  };

  const isEmpty = !isLoading && aboutData && aboutData.panels.length === 0 && aboutData.social_links.length === 0 && !aboutData.description;

  return (
    <div className="flex flex-col h-full">
      {/* Content */}
      <div ref={scrollRef} className="flex-1 overflow-y-auto scrollbar-thin">
        {isLoading && <LoadingSkeleton />}

        {error && (
          <div className="flex-1 flex flex-col items-center justify-center p-8 text-center gap-3 min-h-[50vh]">
            <div className="w-12 h-12 rounded-full bg-red-500/10 flex items-center justify-center">
              <Users size={20} className="text-red-400/60" />
            </div>
            <p className="text-sm text-red-400/80">Failed to load channel info</p>
            <p className="text-xs text-textSecondary/60">The broadcaster might not exist or the channel is banned.</p>
          </div>
        )}

        {isEmpty && <EmptyState />}

        {!isLoading && !error && aboutData && (
          <div className="p-3 space-y-3">
            {/* Hero Section — Avatar, name, followers */}
            {!hideHero && (
              <div className="flex items-center gap-3">
                {aboutData.profile_image_url && (
                  <SevenTvAvatar
                    cosmeticsId={sevenTvIdForStream({ provider: 'twitch', user_id: aboutData.user_id })}
                    px={56}
                    motion="animate"
                    src={aboutData.profile_image_url}
                    alt={aboutData.display_name || channelLogin}
                    className="w-14 h-14 rounded-full object-cover ring-2 ring-borderSubtle/40 flex-shrink-0"
                  />
                )}
                <div className="min-w-0 flex-1">
                  <h3 className="text-base font-bold text-textPrimary truncate">
                    {aboutData.display_name || channelLogin}
                  </h3>
                  {aboutData.follower_count != null && (
                    <div className="flex items-center gap-1.5 mt-0.5">
                      <Users size={11} className="text-textSecondary/70" />
                      <span className="text-xs text-textSecondary">
                        {formatFollowers(aboutData.follower_count)} followers
                      </span>
                    </div>
                  )}
                </div>
              </div>
            )}

            {/* Bio Description */}
            {aboutData.description && (
              <p className="text-xs text-textSecondary leading-relaxed whitespace-pre-wrap">
                {aboutData.description}
              </p>
            )}

            {/* Social links: rich cards where the link has one, branded pills
                for the rest, all opening in the browser. */}
            {aboutData.social_links.length > 0 && <SocialLinks links={aboutData.social_links} />}

            {/* Viewers also watch — live channels adjacent to this one */}
            {similarStreams && similarStreams.length > 0 && (
              <div className="space-y-2">
                <h4 className="text-[11px] font-semibold uppercase tracking-wider text-textSecondary/80 px-1">
                  Viewers also watch
                </h4>
                <div className="flex gap-2 overflow-x-auto scrollbar-none pb-1">
                  {similarStreams.map((stream) => (
                    <button
                      key={stream.user_id || stream.user_login}
                      onClick={() => handleSimilarClick(stream)}
                      className="flex-shrink-0 w-[150px] text-left glass-panel border border-borderSubtle/20 rounded-lg p-2 hover:border-accent/30 hover:bg-white/[0.03] transition-all duration-200 group"
                    >
                      <div className="flex items-center gap-2">
                        {stream.profile_image_url && (
                          <SevenTvAvatar
                            cosmeticsId={sevenTvIdForStream(stream)}
                            px={32}
                            motion="hover"
                            src={stream.profile_image_url}
                            alt=""
                            className="w-8 h-8 rounded-full object-cover flex-shrink-0"
                          />
                        )}
                        <div className="min-w-0">
                          <div className="text-xs font-semibold text-textPrimary truncate group-hover:text-accent transition-colors">
                            {stream.display_name}
                          </div>
                          {stream.game_name && (
                            <div className="text-[10px] text-textSecondary truncate">
                              {stream.game_name}
                            </div>
                          )}
                        </div>
                      </div>
                      <div className="mt-1.5 flex items-center gap-1.5 text-[10px] text-textSecondary">
                        <span className="w-1.5 h-1.5 rounded-full bg-red-500 flex-shrink-0" />
                        {formatViewerCount(stream.viewer_count)} viewers
                      </div>
                    </button>
                  ))}
                </div>
              </div>
            )}

            {/* Divider before panels */}
            {(aboutData.description || aboutData.social_links.length > 0) && aboutData.panels.length > 0 && (
              <div className="border-t border-borderSubtle/30" />
            )}

            {/* Panel Cards */}
            {aboutData.panels.length > 0 && (
              <div className="columns-1 sm:columns-[320px] gap-3 mt-4">
                {aboutData.panels.map((panel) => (
                  <PanelCard key={panel.id} panel={panel} />
                ))}
              </div>
            )}

            {/* Bottom spacer for comfortable scrolling */}
            <div className="h-2" />
          </div>
        )}
      </div>
    </div>
  );
});

export default StreamerAboutPanel;
