import React, { useEffect, useRef, useState, memo } from 'react';
import { Play, ExternalLink, Eye, ShieldCheck } from 'lucide-react';
import { Tooltip, type TooltipChildProps } from '../ui/Tooltip';
import { DiscordGlyph } from '../ui/DiscordGlyph';
import { SocialLogo } from '../ui/SocialLogo';
import { useAppStore, type MediaInfo } from '../../stores/AppStore';
import { playTwitchMediaInMain } from '../../utils/playTwitchMediaInMain';
// Shared helper, not a local copy. The local copy this replaced used
// `@tauri-apps/plugin-shell`, which cannot open anything on Android - and this
// card is the ONLY tap target for trusted domains (Discord, Twitch, YouTube),
// because ChatMessage suppresses the inline anchor for them in the default
// preview mode. So a Discord link on the phone was a dead tap even once the
// inline-anchor path was fixed.
import { openExternal } from '../../utils/openExternal';
import {
  fetchLinkPreview,
  linkPreviewExpected,
  linkPreviewExpectedNow,
  prettyUrlLabel,
  trustableHost,
  type LinkPreview,
} from '../../services/linkPreviewService';
import type { LinkCardShape } from '../../services/twitchChat';

// Twitch clips and VODs deep-link into the in-app player (the user's stated
// preference) rather than opening the browser. start_stream (via playMedia)
// resolves either URL form through the same pipeline a live channel uses. Any
// failure surfaces a toast inside playMedia; the catch here is just a belt-and-
// suspenders fallback to the browser so the link is never dead.
function playTwitchMedia(p: LinkPreview): void {
  const info = {
    id: p.video_id ?? undefined,
    broadcaster_name: p.author ?? undefined,
    user_name: p.author ?? undefined,
    title: p.title ?? undefined,
    view_count: p.view_count ?? undefined,
    thumbnail_url: p.image ?? undefined,
  };
  if (p.kind === 'clip') {
    // A clip opens in the centered overlay modal in THIS window (a clip is a
    // direct MP4, no streaming server), so the current stream/chat stays put
    // underneath and the viewer returns to it on close.
    useAppStore.getState().openClipModal(p.url, info);
  } else {
    void playVod(p.url, info);
  }
}

// VOD playback. A VOD needs the shared HLS relay (unlike a clip's direct MP4). In a
// popout while the main app is CLOSED (chat-only / live mode), play it right here in
// the popout's own lightweight player. Otherwise route to the main player — the
// single shared relay can only serve one stream, and main owns it when it's open.
async function playVod(url: string, info: MediaInfo): Promise<void> {
  const hash = typeof window !== 'undefined' ? window.location.hash : '';
  const isPopout = hash.startsWith('#/multichat') || hash.startsWith('#/profile');
  if (isPopout) {
    try {
      const { getAllWindows } = await import('@tauri-apps/api/window');
      const mainOpen = (await getAllWindows()).some((w) => w.label === 'main');
      if (!mainOpen) {
        useAppStore.getState().openVodModal(url, info);
        return;
      }
    } catch {
      /* fall through to the main player */
    }
  }
  void playTwitchMediaInMain('video', url, info);
}

// The official YouTube glyph (red rounded rect + white play triangle), shown on
// YouTube cards the way the X glyph marks tweet cards.
function YouTubeGlyph({ size = 14 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden className="flex-shrink-0">
      <path
        fill="#FF0000"
        d="M23.498 6.186a3.016 3.016 0 0 0-2.122-2.136C19.505 3.545 12 3.545 12 3.545s-7.505 0-9.377.505A3.017 3.017 0 0 0 .502 6.186C0 8.07 0 12 0 12s0 3.93.502 5.814a3.016 3.016 0 0 0 2.122 2.136c1.871.505 9.376.505 9.376.505s7.505 0 9.377-.505a3.015 3.015 0 0 0 2.122-2.136C24 15.93 24 12 24 12s0-3.93-.502-5.814z"
      />
      <path fill="#fff" d="M9.545 15.568V8.432L15.818 12l-6.273 3.568z" />
    </svg>
  );
}

// The Steam mark, inlined the way the YouTube/Discord glyphs are.
// fill="currentColor" so the surrounding label color drives it.
function SteamGlyph({ size = 14 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor" aria-hidden className="flex-shrink-0">
      <path d="M11.979 0C5.678 0 .511 4.86.022 11.037l6.432 2.658c.545-.371 1.203-.59 1.912-.59.063 0 .125.004.188.006l2.861-4.142V8.91c0-2.495 2.028-4.524 4.524-4.524 2.494 0 4.524 2.031 4.524 4.527s-2.03 4.525-4.524 4.525h-.105l-4.076 2.911c0 .052.004.105.004.159 0 1.875-1.515 3.396-3.39 3.396-1.635 0-3.016-1.173-3.331-2.727L.436 15.27C1.862 20.307 6.486 24 11.979 24c6.627 0 11.999-5.373 11.999-12S18.605 0 11.979 0zM7.54 18.21l-1.473-.61c.262.543.714.999 1.314 1.25 1.297.539 2.793-.076 3.332-1.375.263-.63.264-1.319.005-1.949s-.75-1.121-1.377-1.383c-.624-.26-1.29-.249-1.878-.03l1.523.63c.956.4 1.409 1.5 1.009 2.455-.397.957-1.497 1.41-2.454 1.012zm11.415-9.303c0-1.662-1.353-3.015-3.015-3.015-1.665 0-3.015 1.353-3.015 3.015 0 1.665 1.35 3.015 3.015 3.015 1.663 0 3.015-1.35 3.015-3.015zm-5.273-.005c0-1.252 1.013-2.266 2.265-2.266 1.249 0 2.266 1.014 2.266 2.266 0 1.251-1.017 2.265-2.266 2.265-1.252 0-2.265-1.014-2.265-2.265z" />
    </svg>
  );
}

// The Spotify mark.
function SpotifyGlyph({ size = 14 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor" aria-hidden className="flex-shrink-0">
      <path d="M12 0C5.4 0 0 5.4 0 12s5.4 12 12 12 12-5.4 12-12S18.66 0 12 0zm5.521 17.34c-.24.359-.66.48-1.021.24-2.82-1.74-6.36-2.101-10.561-1.141-.418.122-.779-.179-.899-.539-.12-.421.18-.78.54-.9 4.56-1.021 8.52-.6 11.64 1.32.42.18.479.659.301 1.02zm1.44-3.3c-.301.42-.841.6-1.262.3-3.239-1.98-8.159-2.58-11.939-1.38-.479.12-1.02-.12-1.14-.6-.12-.48.12-1.021.6-1.141C9.6 9.9 15 10.561 18.72 12.84c.361.181.54.78.241 1.2zm.12-3.36C15.24 8.4 8.82 8.16 5.16 9.301c-.6.179-1.2-.181-1.38-.721-.18-.601.18-1.2.72-1.381 4.26-1.26 11.28-1.02 15.721 1.621.539.3.719 1.02.42 1.56-.299.421-1.02.599-1.559.3z" />
    </svg>
  );
}

// Inline -webkit clamp so we don't depend on the Tailwind line-clamp utility
// being enabled in this project's config.
const clamp = (lines: number): React.CSSProperties => ({
  display: '-webkit-box',
  WebkitLineClamp: lines,
  WebkitBoxOrient: 'vertical',
  overflow: 'hidden',
});

function hostOf(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, '');
  } catch {
    return '';
  }
}

// Media length (seconds) -> "0:30" for clips, "1:02:45" for long VODs.
function formatMediaDuration(seconds: number): string {
  const total = Math.max(0, Math.round(seconds));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (h > 0) return `${h}:${m.toString().padStart(2, '0')}:${s.toString().padStart(2, '0')}`;
  return `${m}:${s.toString().padStart(2, '0')}`;
}

// Compact view count: 1234 -> "1.2K", 3_400_000 -> "3.4M".
function formatViews(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, '')}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1).replace(/\.0$/, '')}K`;
  return `${n}`;
}

// --- Fixed card frames ------------------------------------------------------
// Every kind Rust can reserve draws inside a fixed frame: each line sits in a
// line box of its own height, a clamped block always holds all of its lines,
// and a picture has a fixed size or aspect. So a card's height never depends
// on what it says, and the loading skeleton (the same frame with bars in it)
// holds exactly the room the card resolves to. No UI moves down.
const FRAME = 'glass-panel no-live-blur mt-1 block w-full max-w-md overflow-hidden text-left';
const CARD = `${FRAME} cursor-pointer transition-colors hover:bg-white/[0.03]`;
// One 11px line (a byline or site name), and one 14px title line.
const BYLINE = 'flex h-4 min-w-0 items-center gap-1.5 text-[11px] leading-4';
const TITLE = 'flex h-5 min-w-0 items-center text-sm leading-5 text-textPrimary';
// The tweet's text and picture area: two text lines and a picture, or seven
// text lines when there is no picture. Same height either way.
const TWEET_BODY = 152;

/** One line of text that ends in an ellipsis rather than wrapping. */
const Line = ({ children, className = '' }: { children: React.ReactNode; className?: string }) => (
  <span className={`min-w-0 truncate ${className}`}>{children}</span>
);

/** A skeleton's stand-in for a line of text. */
const Bar = ({ w }: { w: string }) => <div className={`h-2.5 ${w} flex-shrink-0 rounded bg-white/5`} />;

/** Skeleton bars filling a fixed multi-line block. */
const Bars = ({ count }: { count: number }) => (
  <div className="flex h-full flex-col justify-center gap-1.5">
    {Array.from({ length: count }, (_, i) => (
      <Bar key={i} w={i === count - 1 ? 'w-1/2' : 'w-full'} />
    ))}
  </div>
);

const PlayBadge = () => (
  <span className="absolute inset-0 flex items-center justify-center">
    <span className="flex h-12 w-12 items-center justify-center rounded-full bg-black/60">
      <Play size={22} className="ml-0.5 text-white" fill="currentColor" />
    </span>
  </span>
);

type Slot = React.ReactNode;

/** youtube, clip, vod: a 16:9 picture, then the byline and ONE title line. */
function VideoShape({ picture, byline, title }: { picture?: Slot; byline?: Slot; title?: Slot }) {
  return (
    <>
      <div className="relative w-full bg-black/40" style={{ aspectRatio: '16 / 9' }}>
        {picture}
      </div>
      <div className="px-3 py-2">
        <div className={`${BYLINE} text-textSecondary`}>{byline}</div>
        <div className={`mt-0.5 ${TITLE} font-medium`}>{title}</div>
      </div>
    </>
  );
}

/** generic: an 80px row, the picture beside the site, one title line and a
 *  two-line description. */
function GenericShape({ picture, site, title, description }: { picture?: Slot; site?: Slot; title?: Slot; description?: Slot }) {
  return (
    <div className="flex h-20 items-center">
      {picture}
      <div className="min-w-0 flex-1 px-3">
        <div className={`${BYLINE} text-textSecondary`}>{site}</div>
        <div className={`${TITLE} font-medium`}>{title}</div>
        <div className="h-7 overflow-hidden text-xs leading-[14px] text-textSecondary">{description}</div>
      </div>
    </div>
  );
}

/** spotify: a 64px row, the cover beside the byline and two title lines. */
function SpotifyShape({ picture, byline, title }: { picture?: Slot; byline?: Slot; title?: Slot }) {
  return (
    <div className="flex h-16 items-center">
      {picture}
      <div className="min-w-0 flex-1 px-3">
        <div className={BYLINE}>{byline}</div>
        <div className="mt-0.5 h-10 overflow-hidden text-sm font-medium leading-5 text-textPrimary">{title}</div>
      </div>
    </div>
  );
}

/** steam: the capsule art at its own aspect, the byline, one title line and a
 *  two-line blurb. */
function SteamShape({ picture, byline, title, description }: { picture?: Slot; byline?: Slot; title?: Slot; description?: Slot }) {
  return (
    <>
      <div className="relative w-full bg-black/40" style={{ aspectRatio: '460 / 215' }}>
        {picture}
      </div>
      <div className="px-3 py-2">
        <div className={BYLINE}>{byline}</div>
        <div className={`mt-0.5 ${TITLE} font-medium`}>{title}</div>
        <div className="mt-0.5 h-8 overflow-hidden text-xs leading-4 text-textSecondary">{description}</div>
      </div>
    </>
  );
}

/** discord: the server icon beside the byline, the name and the counts line,
 *  with the Join pill at the end. */
function DiscordShape({ icon, byline, title, counts, join }: { icon?: Slot; byline?: Slot; title?: Slot; counts?: Slot; join?: Slot }) {
  return (
    <div className="p-3">
      <div className="flex h-14 items-center gap-3">
        {icon}
        <div className="min-w-0 flex-1">
          <div className={BYLINE} style={{ color: '#5865F2' }}>
            {byline}
          </div>
          <div className={`${TITLE} font-semibold`}>{title}</div>
          <div className="mt-1 flex h-4 items-center gap-x-3 overflow-hidden whitespace-nowrap text-[11px] leading-4 text-textSecondary">
            {counts}
          </div>
        </div>
        {join}
      </div>
    </div>
  );
}

/** youtube_channel and instagram: the avatar beside its lines, then a
 *  three-line bio. `headerHeight` fits the lines the kind has. */
function ProfileShape({ avatar, lines, headerHeight, description }: { avatar?: Slot; lines: Slot; headerHeight: number; description?: Slot }) {
  return (
    <div className="p-3">
      <div className="flex items-center gap-2.5" style={{ height: headerHeight }}>
        {avatar}
        <div className="min-w-0 flex-1">{lines}</div>
      </div>
      <div className="mt-2 h-12 overflow-hidden text-xs leading-4 text-textSecondary">{description}</div>
    </div>
  );
}

const XGlyph = () => (
  <svg viewBox="0 0 24 24" className="h-4 w-4 flex-shrink-0 text-textSecondary" fill="currentColor" aria-hidden>
    <path d="M18.244 2.25h3.308l-7.227 8.26 8.502 11.24H16.17l-5.214-6.817L4.99 21.75H1.68l7.73-8.835L1.254 2.25H8.08l4.713 6.231zm-1.161 17.52h1.833L7.084 4.126H5.117z" />
  </svg>
);

/** tweet: the author row, then a fixed text and picture area. */
function TweetShape({ avatar, title, author, body }: { avatar?: Slot; title?: Slot; author?: Slot; body?: Slot }) {
  return (
    <div className="p-3">
      <div className="flex h-9 items-center gap-2">
        {avatar}
        <div className="min-w-0 flex-1">
          <div className={`${TITLE} font-semibold`}>{title}</div>
          <div className="flex h-4 min-w-0 items-center text-xs leading-4 text-textSecondary">{author}</div>
        </div>
        <XGlyph />
      </div>
      <div className="mt-2 overflow-hidden" style={{ height: TWEET_BODY }}>
        {body}
      </div>
    </div>
  );
}

/** The loading card for a reserved kind: that kind's frame with bars in it. */
function ReservedShape({ kind }: { kind: LinkCardShape }) {
  const square = (size: string, round = '') => <div className={`${size} flex-shrink-0 bg-white/5 ${round}`} />;
  switch (kind) {
    case 'youtube':
    case 'clip':
    case 'vod':
      return <VideoShape byline={<Bar w="w-1/3" />} title={<Bar w="w-2/3" />} />;
    case 'spotify':
      return <SpotifyShape picture={square('h-16 w-16')} byline={<Bar w="w-1/3" />} title={<Bars count={2} />} />;
    case 'steam':
      return <SteamShape byline={<Bar w="w-1/4" />} title={<Bar w="w-2/3" />} description={<Bars count={2} />} />;
    case 'discord':
      return (
        <DiscordShape
          icon={square('h-11 w-11', 'rounded-2xl')}
          byline={<Bar w="w-1/4" />}
          title={<Bar w="w-1/2" />}
          counts={<Bar w="w-1/3" />}
          join={square('h-8 w-16', 'rounded-lg')}
        />
      );
    case 'youtube_channel':
      return (
        <ProfileShape
          avatar={square('h-11 w-11', 'rounded-full')}
          headerHeight={44}
          lines={
            <>
              <div className={BYLINE}>
                <Bar w="w-1/4" />
              </div>
              <div className={`mt-0.5 ${TITLE}`}>
                <Bar w="w-1/2" />
              </div>
            </>
          }
          description={<Bars count={3} />}
        />
      );
    case 'instagram':
      return (
        <ProfileShape
          avatar={square('h-11 w-11', 'rounded-full')}
          headerHeight={52}
          lines={
            <>
              <div className={BYLINE}>
                <Bar w="w-1/4" />
              </div>
              <div className={TITLE}>
                <Bar w="w-1/2" />
              </div>
              <div className={BYLINE}>
                <Bar w="w-1/3" />
              </div>
            </>
          }
          description={<Bars count={3} />}
        />
      );
    case 'tweet':
      return (
        <TweetShape
          avatar={square('h-9 w-9', 'rounded-full')}
          title={<Bar w="w-1/3" />}
          author={<Bar w="w-1/4" />}
          body={<Bars count={4} />}
        />
      );
    case 'generic':
    default:
      return (
        <GenericShape
          picture={square('h-20 w-20')}
          site={<Bar w="w-1/4" />}
          title={<Bar w="w-2/3" />}
          description={<Bars count={2} />}
        />
      );
  }
}

/**
 * A single inline link-preview card. Resolves lazily (only fetches once the row
 * scrolls near the viewport) and renders a fixed-height skeleton while loading.
 * Card chrome reuses `.glass-panel` (subtle inset bevel, no outer glow) with
 * `.no-live-blur`: a backdrop filter inside a `content-visibility: auto` chat
 * row leaves stale copies of the card behind as the list scrolls in WebView2.
 *
 * `showChip` controls the no-preview fallback: in "clean" mode (inline link
 * suppressed) it's true, so a failed/empty preview falls back to a compact link
 * chip and the link is never lost. In "keep link" mode it's false — the inline
 * link is still in the message, so a failed preview renders nothing.
 */
export const LinkPreviewCard = memo(function LinkPreviewCard({
  url,
  showChip = true,
  trusted = true,
  resolved,
  expectedCard,
}: {
  url: string;
  showChip?: boolean;
  trusted?: boolean;
  /** An already-fetched preview. The card renders it and fetches nothing. */
  resolved?: LinkPreview;
  /** The card this link's shape gets, as its chat segment says (null: maybe none);
   *  undefined when the caller does not know, and the card asks Rust. */
  expectedCard?: LinkCardShape | null;
}) {
  // undefined = unresolved, null = no preview, object = card. Nothing is cached
  // (chat links are ephemeral), so every card starts unresolved and fetches once.
  const [preview, setPreview] = useState<LinkPreview | null | undefined>(resolved);
  const [imageFailed, setImageFailed] = useState(false);
  // The Reddit card holds the image's full box until it loads, so the card
  // does not grow by the image's height when it arrives.
  const [imageLoaded, setImageLoaded] = useState(false);
  // Trusted links fetch automatically; untrusted links wait for an explicit
  // click (the click-to-load chip below) so an arbitrary pasted link is never
  // fetched passively — that would reveal the user's IP to the linked host.
  const [loadRequested, setLoadRequested] = useState(trusted);
  // The card this link's shape all but always gets (Rust's answer, carried on the
  // chat segment). Only then does the loading card hold that room; any other
  // link shows a card once it has one, so one with no preview never flashes a
  // placeholder and vanishes.
  const [expected, setExpected] = useState<LinkCardShape | null | undefined>(() =>
    expectedCard !== undefined ? expectedCard : linkPreviewExpectedNow(url),
  );
  const containerRef = useRef<HTMLDivElement | null>(null);
  const requested = useRef(resolved !== undefined);

  useEffect(() => {
    if (resolved !== undefined || expected !== undefined) return;
    let cancelled = false;
    void linkPreviewExpected(url).then((answer) => {
      if (!cancelled) setExpected(answer);
    });
    return () => {
      cancelled = true;
    };
  }, [url, resolved, expected]);

  useEffect(() => {
    // Untrusted links don't auto-fetch; loadPreview() drives them on click.
    if (!trusted) return;
    if (requested.current) return;
    const el = containerRef.current;
    if (!el) return;
    let cancelled = false;

    const request = () => {
      if (requested.current) return;
      requested.current = true;
      fetchLinkPreview(url).then((p) => {
        if (!cancelled) setPreview(p ?? null);
      });
    };

    // Defer until near the viewport: a fast chat scrolls links past before the
    // eye lands on them, and there's no point fetching those.
    if (typeof IntersectionObserver === 'undefined') {
      request();
      return () => {
        cancelled = true;
      };
    }
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          observer.disconnect();
          request();
        }
      },
      { rootMargin: '200px' },
    );
    observer.observe(el);
    return () => {
      cancelled = true;
      observer.disconnect();
    };
  }, [url, trusted]);

  // Fetch on demand (untrusted links, when the user clicks the load chip).
  const loadPreview = () => {
    setLoadRequested(true);
    if (requested.current) return;
    requested.current = true;
    fetchLinkPreview(url).then((p) => setPreview(p ?? null));
  };

  // "Always trust this source": persist the host to the user's trusted list so
  // future links from it auto-expand, then load this one. Reads/writes the store
  // imperatively (getState, not a hook) so this hot-path card never subscribes
  // to settings and re-renders on every unrelated settings change.
  const trustHostAndLoad = () => {
    const host = trustableHost(url);
    if (host) {
      const store = useAppStore.getState();
      const cd = store.settings.chat_design;
      const current = cd?.link_preview_trusted_domains ?? [];
      if (!current.includes(host)) {
        store.updateSettings({
          ...store.settings,
          // Preserve every existing chat_design field and only append the host.
          // The cast bridges ChatDesignSettings' required fields: the spread
          // carries over whatever exists, and every reader in the app accesses
          // these with `?? default`, so a partial is tolerated.
          chat_design: {
            ...cd,
            link_preview_trusted_domains: [...current, host],
          } as NonNullable<typeof cd>,
        });
      }
    }
    loadPreview();
  };

  // Twitch clips/VODs open in the in-app player; everything else opens the
  // browser. Reads the resolved `preview` so the right action fires regardless
  // of which card variant rendered (incl. the no-thumbnail generic fallback).
  const activate = () => {
    if (preview && (preview.kind === 'clip' || preview.kind === 'vod')) {
      playTwitchMedia(preview);
    } else {
      void openExternal(url);
    }
  };

  const cardProps = {
    role: 'button' as const,
    tabIndex: 0,
    onClick: activate,
    onKeyDown: (e: React.KeyboardEvent) => {
      if (e.key === 'Enter' || e.key === ' ') {
        e.preventDefault();
        activate();
      }
    },
  };

  // The card IS the link now: the inline URL in the message body is suppressed
  // once a card represents it, so every resolved card opens the link on click
  // and reveals the full URL on hover.
  const wrap = (node: React.ReactElement<TooltipChildProps>) => (
    <Tooltip content={url} side="top">
      {node}
    </Tooltip>
  );

  // Fallback when a link resolves to no usable preview. In "clean" mode
  // (showChip) the inline link was suppressed, so show a compact link chip and
  // never lose the link. In "keep link" mode the inline link is still present,
  // so render nothing.
  const fallback = showChip
    ? wrap(
        <div
          {...cardProps}
          className="glass-panel no-live-blur mt-1 inline-flex max-w-md cursor-pointer items-center gap-1.5 px-2.5 py-1.5 align-middle transition-colors hover:bg-white/[0.03]"
        >
          <ExternalLink size={13} className="flex-shrink-0 text-textSecondary" />
          <span className="truncate text-xs text-blue-400">{prettyUrlLabel(url)}</span>
        </div>,
      )
    : null;

  // Untrusted link, not yet loaded: a click-to-load chip (no passive fetch). The
  // inline link stays in the message body, so this is purely an opt-in preview —
  // clicking loads it, it doesn't navigate. Hover still reveals the full URL. A
  // second chip persists the host to the trusted list ("always trust") so future
  // links from it auto-expand.
  if (!trusted && !loadRequested) {
    return (
      <div className="mt-1 flex max-w-md items-center gap-1.5">
        <Tooltip content={url} side="top">
          <div
            role="button"
            tabIndex={0}
            onClick={loadPreview}
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                loadPreview();
              }
            }}
            className="glass-panel no-live-blur inline-flex min-w-0 cursor-pointer items-center gap-1.5 px-2.5 py-1.5 align-middle transition-colors hover:bg-white/[0.03]"
          >
            <Eye size={13} className="flex-shrink-0 text-textSecondary" />
            <span className="truncate text-xs text-textSecondary">Load preview · {hostOf(url)}</span>
          </div>
        </Tooltip>
        <Tooltip content={`Always trust ${hostOf(url)}`} side="top">
          <button
            type="button"
            onClick={trustHostAndLoad}
            aria-label={`Always trust ${hostOf(url)}`}
            className="glass-panel no-live-blur flex flex-shrink-0 cursor-pointer items-center justify-center p-1.5 text-textSecondary transition-colors hover:bg-white/[0.03] hover:text-textPrimary"
          >
            <ShieldCheck size={14} />
          </button>
        </Tooltip>
      </div>
    );
  }

  // Not resolved yet. A link whose card is all but certain shows a low-profile
  // skeleton holding the card's room; any other link only an invisible line. The
  // same element either way, so it stays the observe target.
  if (preview === undefined) {
    // Clean mode hides the inline link, so a link with no card expected shows
    // its link chip while it loads rather than nothing.
    if (expected === null && showChip) return <div ref={containerRef}>{fallback}</div>;
    // An expected card holds exactly its kind's frame. No card expected: an
    // empty, zero-height observe target, so nothing moves when the answer lands.
    if (!expected) return <div ref={containerRef} className="w-full" style={{ height: 0 }} aria-hidden />;
    return (
      <div ref={containerRef} className={FRAME} style={{ opacity: 0.5 }} aria-hidden>
        <ReservedShape kind={expected} />
      </div>
    );
  }

  if (preview === null) {
    // An untrusted link the user explicitly asked to load resolved to nothing —
    // give a clear, non-vanishing result (the inline link is still in the
    // message) instead of the chip silently disappearing.
    if (!trusted) {
      return (
        <Tooltip content={url} side="top">
          <div className="glass-panel no-live-blur mt-1 inline-flex max-w-md items-center gap-1.5 px-2.5 py-1.5 align-middle text-xs text-textSecondary">
            <Eye size={13} className="flex-shrink-0 opacity-50" />
            <span className="truncate">No preview available · {hostOf(url)}</span>
          </div>
        </Tooltip>
      );
    }
    return fallback;
  }

  const site = preview.site_name || hostOf(url);

  // --- YouTube: thumbnail with a play affordance + title/author -------------
  // A missing or broken thumbnail keeps the 16:9 box, so the card never changes
  // shape (it used to fall through to the generic card).
  if (preview.kind === 'youtube') {
    const showImage = !!preview.image && !imageFailed;
    return wrap(
      <div {...cardProps} className={CARD}>
        <VideoShape
          picture={
            <>
              {showImage && (
                <img
                  src={preview.image as string}
                  alt={preview.title ?? 'YouTube video'}
                  loading="lazy"
                  referrerPolicy="no-referrer"
                  className="h-full w-full object-cover"
                  onError={() => setImageFailed(true)}
                />
              )}
              <PlayBadge />
            </>
          }
          byline={
            <>
              <YouTubeGlyph size={13} />
              <span className="flex-shrink-0 font-semibold text-textPrimary">YouTube</span>
              {preview.author && <Line>· {preview.author}</Line>}
            </>
          }
          title={preview.title && <Line>{preview.title}</Line>}
        />
      </div>,
    );
  }

  // --- YouTube channel / profile: circular avatar + name + bio + glyph ------
  // (Dedicated card so the square avatar renders as a centered circle instead of
  // the generic card's top-pinned left thumbnail.) Opens in the browser.
  if (preview.kind === 'youtube_channel') {
    return wrap(
      <div {...cardProps} className={CARD}>
        <ProfileShape
          headerHeight={44}
          avatar={
            preview.author_avatar && !imageFailed ? (
              <img
                src={preview.author_avatar}
                alt=""
                loading="lazy"
                referrerPolicy="no-referrer"
                className="h-11 w-11 flex-shrink-0 rounded-full object-cover"
                onError={() => setImageFailed(true)}
              />
            ) : undefined
          }
          lines={
            <>
              <div className={`${BYLINE} text-textSecondary`}>
                <YouTubeGlyph size={13} />
                <span className="font-semibold">YouTube</span>
              </div>
              <div className={`mt-0.5 ${TITLE} font-semibold`}>{preview.title && <Line>{preview.title}</Line>}</div>
            </>
          }
          description={preview.description && <div style={clamp(3)}>{preview.description}</div>}
        />
      </div>,
    );
  }

  // --- Instagram profile (via SearchAPI): avatar + name + @handle + bio -------
  if (preview.kind === 'instagram') {
    return wrap(
      <div {...cardProps} className={CARD}>
        <ProfileShape
          headerHeight={52}
          avatar={
            preview.author_avatar && !imageFailed ? (
              <img
                src={preview.author_avatar}
                alt=""
                loading="lazy"
                referrerPolicy="no-referrer"
                className="h-11 w-11 flex-shrink-0 rounded-full object-cover"
                onError={() => setImageFailed(true)}
              />
            ) : undefined
          }
          lines={
            <>
              <div className={`${BYLINE} text-textSecondary`}>
                <SocialLogo platform="instagram" size={13} />
                <span className="flex-shrink-0 font-semibold">Instagram</span>
                {typeof preview.member_count === 'number' && (
                  <Line className="text-textSecondary/70">· {formatViews(preview.member_count)} followers</Line>
                )}
              </div>
              <div className={`${TITLE} font-semibold`}>{preview.title && <Line>{preview.title}</Line>}</div>
              <div className="flex h-4 min-w-0 items-center text-xs leading-4 text-textSecondary">
                {preview.author && <Line>{preview.author}</Line>}
              </div>
            </>
          }
          description={preview.description && <div style={clamp(3)}>{preview.description}</div>}
        />
      </div>,
    );
  }

  // --- Reddit: r/sub · u/author, the post title, Reddit's preview image ------
  // Rust writes every label (site_name is "Reddit" when no subreddit applies).
  if (preview.kind === 'reddit') {
    const showImage = !!preview.image && !imageFailed;
    // A share or video link that did not resolve knows only where it goes:
    // the same compact chip as the other link chips, not an empty card.
    if (!preview.title && !showImage) {
      return wrap(
        <div
          {...cardProps}
          className="glass-panel no-live-blur mt-1 inline-flex max-w-md cursor-pointer items-center gap-1.5 px-2.5 py-1.5 align-middle transition-colors hover:bg-white/[0.03]"
        >
          <SocialLogo platform="reddit" size={13} />
          <span className="truncate text-xs font-semibold text-textPrimary">{preview.site_name}</span>
          {preview.author && <span className="min-w-0 truncate text-xs text-textSecondary/80">· {preview.author}</span>}
        </div>,
      );
    }
    return wrap(
      <div
        {...cardProps}
        className="glass-panel no-live-blur mt-1 block w-full max-w-md cursor-pointer overflow-hidden p-3 text-left transition-colors hover:bg-white/[0.03]"
      >
        <div className="flex min-w-0 items-center gap-1.5 text-[11px] leading-tight text-textSecondary">
          <SocialLogo platform="reddit" size={13} />
          {/* The subreddit keeps its width; the author gives way first. */}
          <span className="max-w-full flex-shrink-0 truncate font-semibold">{preview.site_name}</span>
          {preview.author && <span className="min-w-0 truncate text-textSecondary/80">· {preview.author}</span>}
        </div>
        {preview.title && (
          <div className="mt-1 text-sm font-semibold leading-snug text-textPrimary" style={clamp(3)}>
            {preview.title}
          </div>
        )}
        {showImage && (
          <img
            src={preview.image as string}
            alt=""
            loading="lazy"
            referrerPolicy="no-referrer"
            className={`mt-2 w-full rounded-md bg-white/[0.04] object-cover ${imageLoaded ? 'max-h-72' : 'h-72'}`}
            onLoad={() => setImageLoaded(true)}
            onError={() => setImageFailed(true)}
          />
        )}
      </div>,
    );
  }

  // --- Twitch clip / VOD: 16:9 thumbnail + duration badge + channel/views ----
  // Opens in the in-app player (see activate()). The video frame has one byline,
  // so "Clipped by" and the views ride on it after the channel; a missing
  // thumbnail keeps the 16:9 box.
  if (preview.kind === 'clip' || preview.kind === 'vod') {
    const showImage = !!preview.image && !imageFailed;
    const meta = [
      preview.author,
      preview.description,
      typeof preview.view_count === 'number' ? `${formatViews(preview.view_count)} views` : null,
    ].filter(Boolean);
    return wrap(
      <div {...cardProps} className={CARD}>
        <VideoShape
          picture={
            <>
              {showImage && (
                <img
                  src={preview.image as string}
                  alt={preview.title ?? (preview.kind === 'vod' ? 'Twitch VOD' : 'Twitch clip')}
                  loading="lazy"
                  referrerPolicy="no-referrer"
                  className="h-full w-full object-cover"
                  onError={() => setImageFailed(true)}
                />
              )}
              <PlayBadge />
              {typeof preview.duration === 'number' && preview.duration > 0 && (
                <span className="absolute bottom-1.5 right-1.5 rounded bg-glass-ink/75 px-1.5 py-0.5 text-[11px] font-medium tabular-nums text-white">
                  {formatMediaDuration(preview.duration)}
                </span>
              )}
            </>
          }
          byline={
            <>
              <span className="flex-shrink-0 font-semibold" style={{ color: '#9146FF' }}>
                Twitch
              </span>
              {meta.length > 0 && <Line>· {meta.join(' · ')}</Line>}
            </>
          }
          title={preview.title && <Line>{preview.title}</Line>}
        />
      </div>,
    );
  }

  // --- Discord invite: compact "join this server" card ----------------------
  // Server icon + name + live online/member counts + a blurple Join pill. The
  // whole card opens the invite (the Join pill is a visual affordance, not a
  // nested button), reusing the in-app community card's brand styling.
  if (preview.kind === 'discord') {
    const online = preview.online_count;
    const members = preview.member_count;
    const hasCounts = typeof online === 'number' || typeof members === 'number';
    return wrap(
      <div {...cardProps} className={CARD}>
        <DiscordShape
          icon={
            <div className="h-11 w-11 flex-shrink-0 overflow-hidden rounded-2xl bg-white/[0.04]">
              {preview.image && !imageFailed ? (
                <img
                  src={preview.image}
                  alt=""
                  loading="lazy"
                  referrerPolicy="no-referrer"
                  className="h-full w-full object-cover"
                  onError={() => setImageFailed(true)}
                />
              ) : (
                <div
                  className="flex h-full w-full items-center justify-center text-white"
                  style={{ backgroundColor: '#5865F2' }}
                >
                  <DiscordGlyph size={20} />
                </div>
              )}
            </div>
          }
          byline={
            <>
              <DiscordGlyph size={13} />
              <span className="font-semibold">Discord</span>
            </>
          }
          title={preview.title && <Line>{preview.title}</Line>}
          counts={
            hasCounts && (
              <>
                {/* One line: a count that does not fit ends in an ellipsis. */}
                {typeof online === 'number' && (
                  <span className="flex min-w-0 items-center gap-1.5">
                    <span className="relative flex h-2 w-2 flex-shrink-0">
                      <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-green-500 opacity-60" />
                      <span className="relative inline-flex h-2 w-2 rounded-full bg-green-500" />
                    </span>
                    <Line>
                      <span className="font-semibold text-textPrimary">{online.toLocaleString()}</span> Online
                    </Line>
                  </span>
                )}
                {typeof members === 'number' && (
                  <span className="flex min-w-0 items-center gap-1.5">
                    <span className="h-2 w-2 flex-shrink-0 rounded-full bg-textMuted/50" />
                    <Line>
                      <span className="font-semibold text-textPrimary">{members.toLocaleString()}</span> Members
                    </Line>
                  </span>
                )}
              </>
            )
          }
          join={
            <span className="discord-join-button flex flex-shrink-0 items-center gap-1.5 rounded-lg px-3 py-2 text-[12px] font-semibold text-white">
              <DiscordGlyph size={14} />
              Join
            </span>
          }
        />
      </div>,
    );
  }

  // --- Steam: capsule art + title + price + blurb ---------------------------
  // A missing capsule keeps its box with the Steam mark in it.
  if (preview.kind === 'steam') {
    const showImage = !!preview.image && !imageFailed;
    return wrap(
      <div {...cardProps} className={CARD}>
        <SteamShape
          picture={
            showImage ? (
              <>
                <img
                  src={preview.image as string}
                  alt={preview.title ?? 'Steam'}
                  loading="lazy"
                  referrerPolicy="no-referrer"
                  className="h-full w-full object-cover"
                  onError={() => setImageFailed(true)}
                />
                {preview.author && (
                  <span className="absolute bottom-1.5 right-1.5 rounded bg-glass-ink/75 px-1.5 py-0.5 text-[11px] font-semibold text-white">
                    {preview.author}
                  </span>
                )}
              </>
            ) : (
              <span className="absolute inset-0 flex items-center justify-center" style={{ color: '#66c0f4' }}>
                <SteamGlyph size={28} />
              </span>
            )
          }
          byline={
            <>
              <span className="flex-shrink-0" style={{ color: '#66c0f4' }}>
                <SteamGlyph size={13} />
              </span>
              <span className="flex-shrink-0 font-semibold" style={{ color: '#66c0f4' }}>
                Steam
              </span>
              {!showImage && preview.author && <Line className="text-textSecondary">· {preview.author}</Line>}
            </>
          }
          title={preview.title && <Line>{preview.title}</Line>}
          description={preview.description && <div style={clamp(2)}>{preview.description}</div>}
        />
      </div>,
    );
  }

  // --- Spotify: cover art + title + content type ----------------------------
  // A missing cover keeps its square with the Spotify mark in it.
  if (preview.kind === 'spotify') {
    const showImage = !!preview.image && !imageFailed;
    return wrap(
      <div {...cardProps} className={CARD}>
        <SpotifyShape
          picture={
            showImage ? (
              <img
                src={preview.image as string}
                alt=""
                loading="lazy"
                referrerPolicy="no-referrer"
                className="h-16 w-16 flex-shrink-0 object-cover"
                onError={() => setImageFailed(true)}
              />
            ) : (
              <span
                className="flex h-16 w-16 flex-shrink-0 items-center justify-center"
                style={{ color: '#1DB954', backgroundColor: 'rgba(29, 185, 84, 0.12)' }}
              >
                <SpotifyGlyph size={24} />
              </span>
            )
          }
          byline={
            <>
              <span className="flex-shrink-0" style={{ color: '#1DB954' }}>
                <SpotifyGlyph size={13} />
              </span>
              <span className="flex-shrink-0 font-semibold" style={{ color: '#1DB954' }}>
                Spotify
              </span>
              {preview.description && <Line className="text-textSecondary">· {preview.description}</Line>}
            </>
          }
          title={preview.title && <div style={clamp(2)}>{preview.title}</div>}
        />
      </div>,
    );
  }

  // --- Direct image ---------------------------------------------------------
  if (preview.kind === 'image' && preview.image && !imageFailed) {
    return wrap(
      <div
        {...cardProps}
        className="glass-panel no-live-blur mt-1 inline-block max-w-md cursor-pointer overflow-hidden transition-colors hover:bg-white/[0.03]"
      >
        <img
          src={preview.image}
          alt="Linked image"
          loading="lazy"
          referrerPolicy="no-referrer"
          className="max-h-72 w-auto object-contain"
          onError={() => setImageFailed(true)}
        />
      </div>,
    );
  }

  // --- Tweet / X post ------------------------------------------------------
  if (preview.kind === 'tweet') {
    const showImage = !!preview.image && !imageFailed;
    // The body is one fixed area: two text lines over the picture, or seven
    // text lines when there is none, so a long post or a picture never grows it.
    const text = (lines: number) =>
      preview.description && (
        <div className="whitespace-pre-wrap break-words text-sm leading-5 text-textPrimary" style={clamp(lines)}>
          {preview.description}
        </div>
      );
    return wrap(
      <div {...cardProps} className={CARD}>
        <TweetShape
          avatar={
            preview.author_avatar && (
              <img
                src={preview.author_avatar}
                alt=""
                loading="lazy"
                referrerPolicy="no-referrer"
                className="h-9 w-9 flex-shrink-0 rounded-full object-cover"
                onError={(e) => {
                  e.currentTarget.style.display = 'none';
                }}
              />
            )
          }
          title={preview.title && <Line>{preview.title}</Line>}
          author={preview.author && <Line>{preview.author}</Line>}
          body={
            showImage ? (
              <>
                <div className="h-10 overflow-hidden">{text(2)}</div>
                <img
                  src={preview.image as string}
                  alt=""
                  loading="lazy"
                  referrerPolicy="no-referrer"
                  className="mt-2 w-full rounded-lg object-cover"
                  style={{ height: TWEET_BODY - 48 }}
                  onError={() => setImageFailed(true)}
                />
              </>
            ) : (
              text(7)
            )
          }
        />
      </div>,
    );
  }

  // --- Image-led "media" card (imgur galleries, etc.): big image + title ----
  if (preview.kind === 'media') {
    const showImage = !!preview.image && !imageFailed;
    if (!showImage && !preview.title) return fallback;
    return wrap(
      <div
        {...cardProps}
        className="glass-panel no-live-blur mt-1 block w-full max-w-md cursor-pointer overflow-hidden text-left transition-colors hover:bg-white/[0.03]"
      >
        {showImage && (
          <img
            src={preview.image as string}
            alt={preview.title ?? ''}
            loading="lazy"
            referrerPolicy="no-referrer"
            className="max-h-80 w-full object-cover"
            onError={() => setImageFailed(true)}
          />
        )}
        {(preview.title || site) && (
          <div className="px-3 py-2">
            {site && <div className="truncate text-[11px] text-textSecondary">{site}</div>}
            {preview.title && (
              <div className="text-sm font-medium text-textPrimary" style={clamp(2)}>
                {preview.title}
              </div>
            )}
          </div>
        )}
      </div>,
    );
  }

  // --- Generic OpenGraph card ----------------------------------------------
  const hasText = !!(preview.title || preview.description || site);
  const showImage = !!preview.image && !imageFailed;
  if (!hasText && !showImage) return fallback;

  return wrap(
    <div {...cardProps} className={CARD}>
      <GenericShape
        picture={
          showImage && (
            <img
              src={preview.image as string}
              alt=""
              loading="lazy"
              referrerPolicy="no-referrer"
              className="h-20 w-20 flex-shrink-0 object-cover"
              onError={() => setImageFailed(true)}
            />
          )
        }
        site={site && <Line>{site}</Line>}
        title={preview.title && <Line>{preview.title}</Line>}
        description={preview.description && <div style={clamp(2)}>{preview.description}</div>}
      />
    </div>,
  );
});

export default LinkPreviewCard;
