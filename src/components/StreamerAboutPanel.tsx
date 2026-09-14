import React, { useEffect, useState, memo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ExternalLink, Users, Globe } from 'lucide-react';
import { ChannelAboutData, ChannelPanel as ChannelPanelType } from '../types/panels';
import { Logger } from '../utils/logger';
import { Tooltip } from './ui/Tooltip';
import { useAppStore } from '../stores/AppStore';
import { LinkPreviewCard } from './chat/LinkPreviewCard';
import { isTrustedHost } from '../services/linkPreviewService';
import { formatViewerCount } from '../utils/streamStats';

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
// Social Media SVG Icons (matching Twitch's native icon set)
// ============================================================================

const SOCIAL_SVGS: Record<string, React.ReactNode> = {
  twitter: (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor">
      <path d="M13.903 10.469 21.348 2h-1.764l-6.465 7.353L7.955 2H2l7.808 11.12L2 22h1.764l6.827-7.765L16.046 22H22l-8.097-11.531Zm-2.417 2.748-.791-1.107L4.4 3.3h2.71l5.08 7.11.791 1.107 6.604 9.242h-2.71l-5.389-7.542Z" />
    </svg>
  ),
  youtube: (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor">
      <path fillRule="evenodd" d="M9.792 15.211V8.788L15.436 12l-5.644 3.211Zm12.557-8.444a2.713 2.713 0 0 0-1.91-1.923C18.752 4.391 12 4.391 12 4.391s-6.754 0-8.438.453a2.713 2.713 0 0 0-1.91 1.923C1.2 8.462 1.2 11.999 1.2 11.999s0 3.537.452 5.232c.249.936.98 1.673 1.908 1.923 1.686.456 8.44.456 8.44.456s6.753 0 8.438-.456a2.716 2.716 0 0 0 1.91-1.922C22.8 15.537 22.8 12 22.8 12s0-3.538-.451-5.233Z" clipRule="evenodd" />
    </svg>
  ),
  instagram: (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor">
      <path d="M12 2.163c3.204 0 3.584.012 4.85.07 3.252.148 4.771 1.691 4.919 4.919.058 1.265.069 1.645.069 4.849 0 3.205-.012 3.584-.069 4.849-.149 3.225-1.664 4.771-4.919 4.919-1.266.058-1.644.07-4.85.07-3.204 0-3.584-.012-4.849-.07-3.26-.149-4.771-1.699-4.919-4.92-.058-1.265-.07-1.644-.07-4.849 0-3.204.013-3.583.07-4.849.149-3.227 1.664-4.771 4.919-4.919 1.266-.057 1.645-.069 4.849-.069ZM12 0C8.741 0 8.333.014 7.053.072 2.695.272.273 2.69.073 7.052.014 8.333 0 8.741 0 12c0 3.259.014 3.668.072 4.948.2 4.358 2.618 6.78 6.98 6.98C8.333 23.986 8.741 24 12 24c3.259 0 3.668-.014 4.948-.072 4.354-.2 6.782-2.618 6.979-6.98.059-1.28.073-1.689.073-4.948 0-3.259-.014-3.667-.072-4.947-.196-4.354-2.617-6.78-6.979-6.98C15.668.014 15.259 0 12 0Zm0 5.838a6.162 6.162 0 1 0 0 12.324 6.162 6.162 0 0 0 0-12.324ZM12 16a4 4 0 1 1 0-8 4 4 0 0 1 0 8Zm6.406-11.845a1.44 1.44 0 1 0 0 2.881 1.44 1.44 0 0 0 0-2.881Z" />
    </svg>
  ),
  discord: (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor">
      <path d="M20.317 4.37a19.791 19.791 0 0 0-4.885-1.515.074.074 0 0 0-.079.037c-.21.375-.444.864-.608 1.25a18.27 18.27 0 0 0-5.487 0 12.64 12.64 0 0 0-.617-1.25.077.077 0 0 0-.079-.037A19.736 19.736 0 0 0 3.677 4.37a.07.07 0 0 0-.032.027C.533 9.046-.32 13.58.099 18.057a.082.082 0 0 0 .031.057 19.9 19.9 0 0 0 5.993 3.03.078.078 0 0 0 .084-.028c.462-.63.874-1.295 1.226-1.994a.076.076 0 0 0-.041-.106 13.107 13.107 0 0 1-1.872-.892.077.077 0 0 1-.008-.128 10.2 10.2 0 0 0 .372-.292.074.074 0 0 1 .077-.01c3.928 1.793 8.18 1.793 12.062 0a.074.074 0 0 1 .078.01c.12.098.246.198.373.292a.077.077 0 0 1-.006.127 12.299 12.299 0 0 1-1.873.892.077.077 0 0 0-.041.107c.36.698.772 1.362 1.225 1.993a.076.076 0 0 0 .084.028 19.839 19.839 0 0 0 6.002-3.03.077.077 0 0 0 .032-.054c.5-5.177-.838-9.674-3.549-13.66a.061.061 0 0 0-.031-.03ZM8.02 15.33c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.956-2.419 2.157-2.419 1.21 0 2.176 1.095 2.157 2.42 0 1.333-.956 2.418-2.157 2.418Zm7.975 0c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.955-2.419 2.157-2.419 1.21 0 2.176 1.095 2.157 2.42 0 1.333-.946 2.418-2.157 2.418Z" />
    </svg>
  ),
  tiktok: (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor">
      <path d="M19.59 6.69a4.83 4.83 0 0 1-3.77-4.25V2h-3.45v13.67a2.89 2.89 0 0 1-2.88 2.5 2.89 2.89 0 0 1-2.89-2.89 2.89 2.89 0 0 1 2.89-2.89c.28 0 .54.04.79.1v-3.5a6.37 6.37 0 0 0-.79-.05A6.34 6.34 0 0 0 3.15 15a6.34 6.34 0 0 0 6.34 6.34 6.34 6.34 0 0 0 6.34-6.34V8.46a8.18 8.18 0 0 0 4.76 1.52v-3.4a4.85 4.85 0 0 1-1-.09Z" />
    </svg>
  ),
  facebook: (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor">
      <path d="M24 12.073c0-6.627-5.373-12-12-12s-12 5.373-12 12c0 5.99 4.388 10.954 10.125 11.854v-8.385H7.078v-3.47h3.047V9.43c0-3.007 1.792-4.669 4.533-4.669 1.312 0 2.686.235 2.686.235v2.953H15.83c-1.491 0-1.956.925-1.956 1.874v2.25h3.328l-.532 3.47h-2.796v8.385C19.612 23.027 24 18.062 24 12.073Z" />
    </svg>
  ),
  reddit: (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor">
      <path d="M12 0A12 12 0 0 0 0 12a12 12 0 0 0 12 12 12 12 0 0 0 12-12A12 12 0 0 0 12 0Zm5.01 4.744c.688 0 1.25.561 1.25 1.249a1.25 1.25 0 0 1-2.498.056l-2.597-.547-.8 3.747c1.824.07 3.48.632 4.674 1.488.308-.309.73-.491 1.207-.491.968 0 1.754.786 1.754 1.754 0 .716-.435 1.333-1.01 1.614a3.111 3.111 0 0 1 .042.52c0 2.694-3.13 4.87-7.004 4.87-3.874 0-7.004-2.176-7.004-4.87 0-.183.015-.366.043-.534A1.748 1.748 0 0 1 4.028 12c0-.968.786-1.754 1.754-1.754.463 0 .898.196 1.207.49 1.207-.883 2.878-1.43 4.744-1.487l.885-4.182a.342.342 0 0 1 .14-.197.35.35 0 0 1 .238-.042l2.906.617a1.214 1.214 0 0 1 1.108-.701ZM9.25 12C8.561 12 8 12.562 8 13.25c0 .687.561 1.248 1.25 1.248.687 0 1.248-.561 1.248-1.249 0-.688-.561-1.249-1.249-1.249Zm5.5 0c-.687 0-1.248.561-1.248 1.25 0 .687.561 1.248 1.249 1.248.688 0 1.249-.561 1.249-1.249 0-.687-.562-1.249-1.25-1.249Zm-5.466 3.99a.327.327 0 0 0-.231.094.33.33 0 0 0 0 .463c.842.842 2.484.913 2.961.913.477 0 2.105-.056 2.961-.913a.361.361 0 0 0 .029-.463.33.33 0 0 0-.464 0c-.547.533-1.684.73-2.512.73-.828 0-1.979-.196-2.512-.73a.326.326 0 0 0-.232-.095Z" />
    </svg>
  ),
};

const getSocialSvg = (name: string): React.ReactNode => {
  const key = name.toLowerCase();
  for (const [platform, svg] of Object.entries(SOCIAL_SVGS)) {
    if (key.includes(platform)) return svg;
  }
  // Fallback generic link icon
  return <Globe size={14} />;
};

// ============================================================================
// Subcomponents
// ============================================================================

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
              className="text-xs text-textSecondary/80 leading-relaxed whitespace-pre-wrap break-words [&_a]:text-accent [&_a]:underline [&_a]:underline-offset-2 [&_a:hover]:text-accent/80"
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
  // Social links reuse the chat link-preview system when previews are enabled.
  const linkPreviews = useAppStore((s) => s.settings.chat_design?.link_previews ?? true);
  const trustedDomains = useAppStore((s) => s.settings.chat_design?.link_preview_trusted_domains);

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
                  <img
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

            {/* Social Links — through the chat link-preview system when previews
                are on (rich cards for trusted hosts, click-to-load chips for the
                rest so the link is never lost); compact platform pills otherwise. */}
            {aboutData.social_links.length > 0 && (
              linkPreviews ? (
                <div className="columns-1 sm:columns-[300px] gap-3">
                  {aboutData.social_links.map((link, i) => {
                    let trusted = false;
                    try {
                      trusted = isTrustedHost(new URL(link.url).hostname, trustedDomains ?? []);
                    } catch {
                      trusted = false;
                    }
                    return (
                      <div key={i} className="mb-3 break-inside-avoid">
                        <LinkPreviewCard url={link.url} trusted={trusted} showChip />
                      </div>
                    );
                  })}
                </div>
              ) : (
                <div className="flex flex-wrap gap-2">
                  {aboutData.social_links.map((link, i) => (
                    <Tooltip key={i} content={link.url} side="top">
                      <a
                        href={link.url}
                        target="_blank"
                        rel="noopener noreferrer"
                        onClick={(e) => {
                          e.preventDefault();
                          invoke('open_browser_url', { url: link.url });
                        }}
                        className="inline-flex items-center gap-2.5 px-3 py-2 rounded-lg glass-panel border border-borderSubtle/20 text-textSecondary hover:text-textPrimary hover:border-accent/30 hover:bg-white/[0.03] transition-all duration-200 group flex-auto min-w-[140px] max-w-full sm:max-w-[240px]"
                      >
                        <span className="text-textSecondary/70 group-hover:text-accent transition-colors flex-shrink-0">
                          {getSocialSvg(link.name)}
                        </span>
                        <span className="text-xs font-medium flex-1 truncate">{link.title}</span>
                        <ExternalLink size={10} className="text-textSecondary/30 group-hover:text-textSecondary/60 transition-colors flex-shrink-0" />
                      </a>
                    </Tooltip>
                  ))}
                </div>
              )
            )}

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
                          <img
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
