import { memo, useEffect, useReducer, type CSSProperties } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowUpRight } from 'lucide-react';
import type { SocialMediaLink } from '../types/panels';
import { LinkPreviewCard } from './chat/LinkPreviewCard';
import { fetchLinkPreview, prettyUrlLabel, type LinkPreview, type LinkPreviewKind } from '../services/linkPreviewService';
import { Tooltip } from './ui/Tooltip';
import { SocialLogo } from './ui/SocialLogo';

// Preview kinds that are a real card (a Discord server, a YouTube channel, an X
// or Instagram profile, ...). A link that only resolves to the generic page
// scrape says nothing a pill does not, so it stays a pill.
const CARD_KINDS: ReadonlySet<LinkPreviewKind> = new Set([
  'youtube', 'youtube_channel', 'tweet', 'instagram', 'discord', 'steam', 'spotify', 'clip', 'vod',
]);

// Session-lifetime, like the About panel's "Viewers also watch" cache: the
// drawer reopens constantly and a channel's links rarely change. null = the
// link has no card, so it is drawn as a pill.
const cardCache = new Map<string, LinkPreview | null>();

/** The colour a link's pill tints toward on hover. Plain-black brands use the
 *  theme's text colour, matching how their mark is drawn. */
function socialBrandColor(platform: string): string {
  switch (platform) {
    case 'instagram': return '#E1306C';
    case 'youtube': return '#FF0000';
    case 'tiktok': return '#FE2C55';
    case 'discord': return '#5865F2';
    case 'reddit': return '#FF4500';
    case 'facebook': return '#0866FF';
    case 'twitch': return '#9146FF';
    case 'kick': return '#53FC19';
    case 'snapchat': return '#FFFC00';
    case 'spotify': return '#1ED760';
    case 'linktree': return '#43E55E';
    case 'kofi': return '#FF6433';
    case 'bluesky': return '#0285FF';
    default: return 'var(--color-text-primary)';
  }
}

const toCard = (p: LinkPreview | null): LinkPreview | null => (p && CARD_KINDS.has(p.kind) ? p : null);

const CardSkeleton = () => (
  <div
    className="glass-panel no-live-blur mt-1 w-full max-w-md overflow-hidden animate-pulse"
    style={{ height: 68 }}
    aria-hidden
  >
    <div className="flex h-full items-center gap-3 px-3">
      <div className="h-11 w-11 flex-shrink-0 rounded-full bg-white/5" />
      <div className="flex-1 space-y-2">
        <div className="h-2.5 w-1/4 rounded bg-white/5" />
        <div className="h-2.5 w-1/2 rounded bg-white/5" />
      </div>
    </div>
  </div>
);

const SocialPill = memo(({ link }: { link: SocialMediaLink }) => (
  <Tooltip content={prettyUrlLabel(link.url, 60)} side="top">
    <a
      href={link.url}
      onClick={(e) => {
        e.preventDefault();
        invoke('open_browser_url', { url: link.url });
      }}
      style={{ '--sn-brand': socialBrandColor(link.platform) } as CSSProperties}
      className="sn-social-pill group glass-panel no-live-blur inline-flex h-9 max-w-[260px] items-center gap-2 rounded-xl pl-2.5 pr-3 text-textPrimary"
    >
      <SocialLogo platform={link.platform} size={18} className="text-textPrimary" />
      <span className="truncate text-[13px] font-medium">{link.title}</span>
      <ArrowUpRight
        size={13}
        className="-ml-0.5 flex-shrink-0 text-textSecondary opacity-0 transition-opacity duration-150 group-hover:opacity-100"
        aria-hidden
      />
    </a>
  </Tooltip>
));

/**
 * A channel's social links. A link Rust marks `card` (its URL has a dedicated
 * preview card) is drawn as that card once it resolves; everything else, and
 * any card that fails to resolve, is a branded pill. Both open in the browser.
 */
export const SocialLinks = memo(({ links }: { links: SocialMediaLink[] }) => {
  // The cache is the state; this only asks for a redraw when an answer lands.
  const [, redraw] = useReducer((n: number) => n + 1, 0);

  useEffect(() => {
    let cancelled = false;
    for (const link of links) {
      if (!link.card || cardCache.has(link.url)) continue;
      void fetchLinkPreview(link.url).then((p) => {
        cardCache.set(link.url, toCard(p));
        if (!cancelled) redraw();
      });
    }
    return () => {
      cancelled = true;
    };
  }, [links]);

  // A pending card (not in the cache yet) holds its place as a skeleton; one
  // that came back empty drops to the pill row.
  const cardLinks = links.filter((l) => l.card && cardCache.get(l.url) !== null);
  const pillLinks = links.filter((l) => !l.card || cardCache.get(l.url) === null);

  return (
    <div className="space-y-3">
      {cardLinks.length > 0 && (
        <div className="grid grid-cols-[repeat(auto-fill,minmax(280px,1fr))] gap-x-3 gap-y-2">
          {cardLinks.map((link) => {
            const preview = cardCache.get(link.url);
            // Cards sharing a row share its height, so the row has one edge.
            return (
              <div key={link.url} className="flex min-w-0 flex-col [&>*]:max-w-none [&>*]:flex-1">
                {preview ? <LinkPreviewCard url={link.url} resolved={preview} trusted showChip /> : <CardSkeleton />}
              </div>
            );
          })}
        </div>
      )}
      {pillLinks.length > 0 && (
        <div className="flex flex-wrap gap-2">
          {pillLinks.map((link) => (
            <SocialPill key={link.url} link={link} />
          ))}
        </div>
      )}
    </div>
  );
});

export default SocialLinks;
