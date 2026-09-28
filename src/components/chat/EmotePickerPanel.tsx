// The shared emote picker, lifted out of ChatWidget so the main chat composer
// and the mod-room composer render the SAME picker (provider tabs, favorites,
// emoji, lazy-mounted grids) plus the swapping-smiley trigger. The host owns the
// open state and provides a `relative` ancestor for the popover to anchor to.

import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode, RefObject } from 'react';
import { motion } from 'framer-motion';
import { Lock, Settings } from 'lucide-react';
import { Tooltip } from '../ui/Tooltip';
import EmoteProviderLogo from './EmoteProviderLogo';
import {
  type Emote,
  type EmoteSet,
  getCachedEmoteUrl,
  queueEmoteForDisplayCaching,
  setEmoteCacheBurst,
  inlineEmoteTier,
  sevenTvTierUrl,
} from '../../services/emoteService';
import {
  loadFavoriteEmotes,
  isFavoriteEmote,
  getAvailableFavorites,
  addFavoriteEmote,
  removeFavoriteEmote,
} from '../../services/favoriteEmoteService';
// The emoji dataset is ~110KB of source; it hydrates on first picker open so
// it rides its own chunk instead of the boot bundle.
type EmojiData = {
  EMOJI_CATEGORIES: Record<string, string[]>;
  EMOJI_KEYWORDS: Record<string, string[]>;
};
let emojiDataCache: EmojiData | null = null;
import { getAppleEmojiUrl } from '../../services/emojiService';
import {
  getGifPickerStatus,
  searchGifs,
  sendGifMessage,
  gifSendMessage,
  type GifItem,
  type GifPickerStatus,
} from '../../services/gifService';
import { useAppStore } from '../../stores/AppStore';
import { Logger } from '../../utils/logger';
import { MOD_PREFIX, staticModifierStyle } from '../../utils/emoteModifiers';
import { PROVIDERS } from '../../types/providers';
import { IS_MOBILE } from '../../utils/platform';

type ProviderTab = 'twitch' | 'bttv' | '7tv' | 'ffz' | 'favorites' | 'emoji' | 'kick' | 'youtube' | 'gifs';

// ── swapping smiley (shared trigger icon) ────────────────────────────────────
const SMILEY_POOL = ['😀', '😄', '😁', '😆', '🤣', '😂', '😊', '😇', '🙂', '😉', '😌', '😍', '🥰', '😜', '🤪', '😎', '🤩', '🥳', '😏', '😋', '🤗', '🫠', '🫡', '😺'];

export function useSwappingSmiley() {
  const [currentSmiley, setCurrentSmiley] = useState('😀');
  const [isSmileyTransitioning, setIsSmileyTransitioning] = useState(false);
  const cycleEmoteSmiley = useCallback(() => {
    setIsSmileyTransitioning(true);
    setTimeout(() => {
      setCurrentSmiley((prev) => {
        const filtered = SMILEY_POOL.filter((s) => s !== prev);
        return filtered[Math.floor(Math.random() * filtered.length)];
      });
      setIsSmileyTransitioning(false);
    }, 110);
  }, []);
  return { currentSmiley, isSmileyTransitioning, cycleEmoteSmiley };
}

// ── grid item ────────────────────────────────────────────────────────────────
const EmoteGridItem = memo(
  ({
    emote,
    isFavorited,
    onInsert,
    onToggleFavorite,
  }: {
    emote: Emote;
    isFavorited: boolean;
    onInsert: () => void;
    onToggleFavorite: () => void;
  }) => {
    const is7tv = emote.provider === '7tv';
    const ffzIsSubwoofer = useAppStore((s) => s.ffzIsSubwoofer);
    const isModifier = emote.modifierFlags != null;
    // BetterTTV modifiers attach to the emote after them, FFZ ones to the
    // emote before, so the hint has to say which.
    const isPrefixModifier = ((emote.modifierFlags ?? 0) & MOD_PREFIX) !== 0;
    // Visible but not composable. Two independent sources, one treatment:
    // subscriber-only FFZ effects (incoming messages still render for everyone),
    // and any emote the provider marked locked for this account (YouTube
    // members-only emoji).
    const lockedSub = (!!emote.ffzSubOnly && !ffzIsSubwoofer) || !!emote.locked;
    const emoteTier = inlineEmoteTier();
    const liveLocal = getCachedEmoteUrl(emote.id, emote.provider, emoteTier);
    const gridSrc = is7tv
      ? liveLocal || emote.localUrl || sevenTvTierUrl(emote.id, emoteTier)
      : liveLocal || emote.localUrl || emote.url;
    const hoverPreviewSize = useAppStore((s) => s.settings.chat_design?.emote_hover_size) ?? 96;

    return (
      <Tooltip
        side="top"
        delay={200}
        content={
          <div className="flex flex-col items-center gap-1.5 py-0.5">
            <img
              src={emote.provider === '7tv' ? `https://cdn.7tv.app/emote/${emote.id}/4x.avif` : emote.localUrl || emote.url}
              alt={emote.name}
              className="w-auto object-contain mx-auto drop-shadow-md"
              style={{ height: hoverPreviewSize, maxWidth: hoverPreviewSize * 2, ...staticModifierStyle(emote.modifierFlags) }}
              onError={(e) => {
                const t = e.currentTarget;
                if (emote.provider === '7tv') {
                  const ladder = ['4x', '3x', '2x', '1x'].flatMap((s) => [
                    `https://cdn.7tv.app/emote/${emote.id}/${s}.avif`,
                    `https://cdn.7tv.app/emote/${emote.id}/${s}.webp`,
                  ]);
                  let step = Number(t.dataset.fb || '0');
                  while (step < ladder.length && ladder[step] === t.src) step++;
                  if (step < ladder.length) {
                    t.dataset.fb = String(step + 1);
                    t.src = ladder[step];
                    return;
                  }
                  if (emote.localUrl && t.src !== emote.localUrl) t.src = emote.localUrl;
                }
              }}
            />
            <div className="text-center flex flex-col items-center gap-0.5">
              <span className="font-bold text-[13px] leading-tight">{emote.name}</span>
              <span className="text-[10px] text-white/60 leading-tight">
                {emote.owner_name ? `by ${emote.owner_name}` : emote.provider}
              </span>
              {isModifier ? (
                <span className="text-[9px] font-bold tracking-wider uppercase text-purple-300 mt-0.5 mix-blend-screen drop-shadow-sm">
                  {isPrefixModifier
                    ? 'Modifier - applies to the next emote'
                    : 'Modifier - applies to the previous emote'}
                </span>
              ) : (
                emote.isZeroWidth && (
                  <span className="text-[9px] font-bold tracking-wider uppercase text-yellow-400 mt-0.5 mix-blend-screen drop-shadow-sm">
                    Zero-Width
                  </span>
                )
              )}
              {emote.ffzSubOnly && (
                <span className={`text-[9px] font-bold tracking-wider uppercase mt-0.5 mix-blend-screen drop-shadow-sm ${lockedSub ? 'text-white/50' : 'text-emerald-300'}`}>
                  {lockedSub ? 'FFZ subscriber effect - locked' : 'FFZ subscriber effect'}
                </span>
              )}
              {emote.locked && emote.lockedLabel && (
                <span className="text-[9px] font-bold tracking-wider uppercase mt-0.5 mix-blend-screen drop-shadow-sm text-white/50">
                  {emote.lockedLabel}
                </span>
              )}
            </div>
          </div>
        }
      >
        <div
          className="relative group flex items-center justify-center focus:outline-none w-full h-full min-h-8"
          style={{ contentVisibility: 'auto', containIntrinsicBlockSize: '40px' }}
        >
          <button
            onClick={onInsert}
            disabled={lockedSub}
            className={`flex items-center justify-center p-1 w-full h-full min-w-8 min-h-8 hover:bg-glass rounded transition-colors ${
              isModifier
                ? 'ring-1 ring-purple-400/50 bg-purple-400/10'
                : emote.isZeroWidth
                  ? 'ring-1 ring-yellow-400/50 bg-yellow-400/10'
                  : ''
            } ${lockedSub ? 'opacity-40 cursor-not-allowed' : ''}`}
          >
            <img
              src={gridSrc}
              srcSet={is7tv && !emote.localUrl ? `https://cdn.7tv.app/emote/${emote.id}/1x.avif 1x, https://cdn.7tv.app/emote/${emote.id}/2x.avif 2x` : undefined}
              alt={emote.name}
              loading="lazy"
              decoding="async"
              referrerPolicy="no-referrer"
              className={`max-h-8 w-auto max-w-full object-contain ${
                isModifier
                  ? 'drop-shadow-[0_0_3px_color-mix(in_srgb,#c084fc_60%,transparent)]'
                  : emote.isZeroWidth
                    ? 'drop-shadow-[0_0_3px_color-mix(in_srgb,var(--color-warning)_60%,transparent)]'
                    : ''
              }`}
              onError={(e) => {
                const t = e.currentTarget;
                if (is7tv) {
                  t.srcset = '';
                  const ladder = ['2x', '1x', '3x', '4x'].flatMap((s) => [
                    `https://cdn.7tv.app/emote/${emote.id}/${s}.avif`,
                    `https://cdn.7tv.app/emote/${emote.id}/${s}.webp`,
                  ]);
                  let step = Number(t.dataset.fb || '0');
                  while (step < ladder.length && ladder[step] === t.src) step++;
                  if (step < ladder.length) {
                    t.dataset.fb = String(step + 1);
                    t.src = ladder[step];
                    return;
                  }
                  t.style.opacity = '0.3';
                  return;
                }
                if (emote.localUrl && t.src !== emote.url) t.src = emote.url;
                else t.style.opacity = '0.3';
              }}
            />
          </button>
          {lockedSub && (
            <Lock className="absolute bottom-0.5 right-0.5 w-3 h-3 text-white/60 pointer-events-none" />
          )}
          <Tooltip content={isFavorited ? 'Remove from favorites' : 'Add to favorites'}>
            <button
              onClick={(e) => {
                e.stopPropagation();
                onToggleFavorite();
              }}
              className={`absolute top-0 right-0 p-1 rounded-bl transition-all ${isFavorited ? 'text-yellow-400 opacity-100' : 'text-textSecondary opacity-0 group-hover:opacity-100'} hover:text-yellow-400 hover:bg-glass`}
            >
              <svg className="w-3 h-3" fill={isFavorited ? 'currentColor' : 'none'} stroke="currentColor" strokeWidth={2} viewBox="0 0 20 20">
                <path d="M9.049 2.927c.3-.921 1.603-.921 1.902 0l1.07 3.292a1 1 0 00.95.69h3.462c.969 0 1.371 1.24.588 1.81l-2.8 2.034a1 1 0 00-.364 1.118l1.07 3.292c.3.921-.755 1.688-1.54 1.118l-2.8-2.034a1 1 0 00-1.175 0l-2.8 2.034c-.784.57-1.838-.197-1.539-1.118l1.07-3.292a1 1 0 00-.364-1.118L2.98 8.72c-.783-.57-.38-1.81.588-1.81h3.461a1 1 0 00.951-.69l1.07-3.292z" />
              </svg>
            </button>
          </Tooltip>
        </div>
      </Tooltip>
    );
  },
);

function chunkArray<T>(arr: T[], size: number): T[][] {
  if (size <= 0) return [arr];
  const out: T[][] = [];
  for (let i = 0; i < arr.length; i += size) out.push(arr.slice(i, i + size));
  return out;
}

const WIDTH_BLOCK_ROWS = 8;
const WIDTH_ROW_PX = 52;
const TWITCH_BLOCK_ROWS = 6;
// Desktop keeps the name label under each Twitch / Kick emote (60 px rows).
// The phone drops the label so the grid fits the narrow picker: image plus
// padding only, matching EmoteGridItem, which never had a label and puts the
// name in the tooltip.
const TWITCH_ROW_PX = IS_MOBILE ? 42 : 60;
const TWITCH_COLS = 7;

const LazyEmoteBlock = memo(
  ({
    scrollRef,
    estimatedHeight,
    gridClass,
    onActivate,
    children,
  }: {
    scrollRef: RefObject<HTMLDivElement | null>;
    estimatedHeight: number;
    gridClass: string;
    onActivate?: () => void;
    children: () => ReactNode;
  }) => {
    const ref = useRef<HTMLDivElement>(null);
    const [visible, setVisible] = useState(false);
    const activatedRef = useRef(false);
    useEffect(() => {
      const el = ref.current;
      const root = scrollRef.current;
      if (!el || !root) return;
      let timer: ReturnType<typeof setTimeout> | undefined;
      const obs = new IntersectionObserver(
        (entries) => {
          const intersecting = entries[0]?.isIntersecting ?? false;
          if (timer) clearTimeout(timer);
          if (intersecting) {
            timer = setTimeout(() => {
              setVisible(true);
              if (!activatedRef.current) {
                activatedRef.current = true;
                onActivate?.();
              }
            }, 80);
          } else {
            timer = setTimeout(() => setVisible(false), 500);
          }
        },
        { root, rootMargin: '600px 0px' },
      );
      obs.observe(el);
      return () => {
        obs.disconnect();
        if (timer) clearTimeout(timer);
      };
      // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [scrollRef]);
    return (
      <div ref={ref} className={visible ? gridClass : undefined} style={visible ? undefined : { minHeight: estimatedHeight }}>
        {visible ? children() : null}
      </div>
    );
  },
);

// ── the panel ────────────────────────────────────────────────────────────────
export interface EmotePickerPanelProps {
  open: boolean;
  onClose: () => void;
  emotes: EmoteSet | null;
  isTwitch: boolean;
  isKick: boolean;
  /** YouTube chat: shows the emoji learned from this room's messages. */
  isYouTube?: boolean;
  channelId?: string;
  channelLogin?: string;
  isLoadingEmotes?: boolean;
  channelNameCache?: Map<string, string>;
  onInsert: (text: string) => void;
  onManageEmotes?: () => void;
  /** Positioning for the popover (defaults to full-width above the composer). */
  className?: string;
}

const DEFAULT_PANEL_CLASS =
  'absolute bottom-full left-0 right-0 mb-2 h-[520px] max-h-[calc(100vh-120px)] border border-borderSubtle rounded-lg shadow-lg flex flex-col overflow-hidden origin-bottom';

export function EmotePickerPanel({
  open,
  onClose,
  emotes,
  isTwitch,
  isKick,
  isYouTube = false,
  channelId,
  channelLogin: _channelLogin,
  isLoadingEmotes = false,
  channelNameCache,
  onInsert,
  onManageEmotes,
  className,
}: EmotePickerPanelProps) {
  const [mounted, setMounted] = useState(open);
  const [fullyClosed, setFullyClosed] = useState(!open);
  // Opening mounts the panel and cancels "fully closed" during render
  // (adjust-state-on-prop-change), so the first open frame already has it.
  const [seenOpen, setSeenOpen] = useState(open);
  if (open !== seenOpen) {
    setSeenOpen(open);
    if (open) {
      setMounted(true);
      setFullyClosed(false);
    }
  }
  const [selectedProvider, setSelectedProvider] = useState<ProviderTab>(
    isTwitch ? 'twitch' : isKick ? 'kick' : isYouTube ? 'youtube' : 'emoji',
  );
  const [searchQuery, setSearchQuery] = useState('');
  const [favoriteEmotes, setFavoriteEmotes] = useState<Emote[]>([]);
  const scrollRef = useRef<HTMLDivElement>(null);

  // ── Twitch chat GIFs ───────────────────────────────────────────────────────
  // Eligibility is Twitch's call (the server-side Tier 2/3 gate), so the tab
  // asks once per channel and renders what it is told rather than guessing.
  // Picking a GIF SENDS it, like Twitch's own keyboard: there is no text form
  // to insert into the composer, the asset only exists as a `gifs` tag Twitch
  // stamps on the outgoing message.
  // Both bits of GIF state are stored WITH the key they belong to, and the key
  // is compared during render. That keeps every write asynchronous (no
  // setState inside an effect body, which cascades renders) and makes staleness
  // impossible by construction rather than by a request-id guard: results for a
  // channel or query you have moved on from simply do not match the key.
  const [gifStatusFor, setGifStatusFor] = useState<{ key: string; status: GifPickerStatus } | null>(null);
  const [gifResults, setGifResults] = useState<{ key: string; items: GifItem[] } | null>(null);
  const [gifNotice, setGifNotice] = useState('');
  const [sendingGifId, setSendingGifId] = useState<string | null>(null);

  const gifStatus = gifStatusFor && gifStatusFor.key === channelId ? gifStatusFor.status : null;
  const gifQueryKey = `${channelId ?? ''}|${searchQuery}`;
  // null means "no results for THIS key yet", which is what renders the
  // loading state; an empty array means the search genuinely returned nothing.
  const gifs = gifResults && gifResults.key === gifQueryKey ? gifResults.items : null;

  // Ask once per channel, and only where GIFs can exist: Twitch, with an id,
  // while the picker is actually open. Nothing runs for a closed picker.
  useEffect(() => {
    if (!open || !isTwitch || !channelId) return;
    let cancelled = false;
    void getGifPickerStatus(channelId)
      .then((status) => { if (!cancelled) setGifStatusFor({ key: channelId, status }); })
      .catch(() => { if (!cancelled) setGifStatusFor(null); });
    return () => { cancelled = true; };
  }, [open, isTwitch, channelId]);

  // Trending on open, debounced search as you type.
  useEffect(() => {
    if (selectedProvider !== 'gifs' || !channelId || !gifStatus?.can_use) return;
    let cancelled = false;
    const t = setTimeout(() => {
      void searchGifs(channelId, searchQuery)
        .then((items) => { if (!cancelled) setGifResults({ key: gifQueryKey, items }); })
        .catch(() => {
          if (cancelled) return;
          setGifResults({ key: gifQueryKey, items: [] });
          setGifNotice('GIF search is unavailable right now.');
        });
    }, searchQuery ? 250 : 0);
    return () => { cancelled = true; clearTimeout(t); };
  }, [selectedProvider, searchQuery, channelId, gifQueryKey, gifStatus?.can_use]);

  const onPickGif = useCallback(
    async (gif: GifItem) => {
      if (!channelId || sendingGifId) return;
      setSendingGifId(gif.id);
      setGifNotice('');
      try {
        const outcome = await sendGifMessage(channelId, gif, searchQuery);
        if (outcome.sent) {
          onClose();
        } else {
          setGifNotice(gifSendMessage(outcome));
        }
      } catch {
        setGifNotice('That GIF could not be sent.');
      } finally {
        setSendingGifId(null);
      }
    },
    [channelId, searchQuery, sendingGifId, onClose],
  );

  // Aggressive disk caching while the picker is open; polite trickle on close.
  useEffect(() => {
    if (!open) return;
    setEmoteCacheBurst(true);
    return () => setEmoteCacheBurst(false);
  }, [open]);

  // Load favorites once the picker opens.
  useEffect(() => {
    if (!open) return;
    loadFavoriteEmotes().then(() => {
      if (emotes) {
        const all = [...emotes.twitch, ...emotes.bttv, ...emotes['7tv'], ...emotes.ffz, ...emotes.kick, ...emotes.youtube];
        setFavoriteEmotes(getAvailableFavorites(all));
      }
    });
  }, [open, emotes]);

  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = 0;
  }, [selectedProvider, searchQuery]);

  const [emojiData, setEmojiData] = useState<EmojiData | null>(emojiDataCache);
  useEffect(() => {
    if (!open || emojiData) return;
    let alive = true;
    void import('../../services/emojiCategories').then((mod) => {
      emojiDataCache = mod;
      if (alive) setEmojiData(mod);
    });
    return () => {
      alive = false;
    };
  }, [open, emojiData]);

  const allEmojis = useMemo(
    () =>
      emojiData
        ? Object.entries(emojiData.EMOJI_CATEGORIES).flatMap(([category, emojis]) =>
            emojis.map((emoji) => ({ emoji, category })),
          )
        : [],
    [emojiData],
  );

  const filteredEmotes = useMemo((): Emote[] => {
    // Emoji and GIFs are not EmoteSet-backed: both render their own pane.
    if (selectedProvider === 'emoji' || selectedProvider === 'gifs') return [];
    if (selectedProvider === 'favorites') {
      if (!searchQuery) return favoriteEmotes;
      const query = searchQuery.toLowerCase();
      return favoriteEmotes.filter((e) => e.name.toLowerCase().includes(query));
    }
    if (!emotes) return [];
    const providerEmotes = emotes[selectedProvider] || [];
    if (!searchQuery) return providerEmotes;
    const query = searchQuery.toLowerCase();
    return providerEmotes.filter((e) => e.name.toLowerCase().includes(query));
  }, [selectedProvider, favoriteEmotes, searchQuery, emotes]);

  const groupedWidthEmotes = useMemo(() => {
    const groups = new Map<string, { label: string; emotes: Emote[]; gridCols: string; cols: number }>();
    groups.set('standard', { label: 'Standard', emotes: [], gridCols: 'grid-cols-7', cols: 7 });
    groups.set('wide', { label: 'Wide', emotes: [], gridCols: 'grid-cols-4', cols: 4 });
    groups.set('ultrawide', { label: 'Ultra Wide', emotes: [], gridCols: 'grid-cols-3', cols: 3 });
    for (const emote of filteredEmotes) {
      const width = emote.width || 32;
      if (width <= 48) groups.get('standard')!.emotes.push(emote);
      else if (width <= 80) groups.get('wide')!.emotes.push(emote);
      else groups.get('ultrawide')!.emotes.push(emote);
    }
    for (const group of groups.values()) {
      group.emotes.sort((a, b) => {
        if (a.isZeroWidth && !b.isZeroWidth) return -1;
        if (!a.isZeroWidth && b.isZeroWidth) return 1;
        const wA = a.width || 32;
        const wB = b.width || 32;
        if (wA !== wB) return wA - wB;
        return a.name.localeCompare(b.name);
      });
    }
    return groups;
  }, [filteredEmotes]);

  const groupedTwitchEmotes = useMemo((): Map<string, { name: string; emotes: Emote[] }> => {
    const groups = new Map<string, { name: string; emotes: Emote[] }>();
    for (const emote of filteredEmotes) {
      const type = emote.emote_type || 'globals';
      const ownerId = emote.owner_id || 'twitch';
      let groupKey: string;
      let groupName: string;
      if (type === 'globals' || !emote.owner_id) {
        groupKey = 'globals';
        groupName = 'Global Emotes';
      } else if (type === 'subscriptions') {
        groupKey = `sub-${ownerId}`;
        groupName = channelNameCache?.get(ownerId) || `Channel ${ownerId}`;
      } else if (type === 'bitstier') {
        groupKey = 'bits';
        groupName = 'Bits Emotes';
      } else if (type === 'follower') {
        groupKey = `follower-${ownerId}`;
        groupName = 'Follower Emotes';
      } else if (type === 'channelpoints') {
        groupKey = `points-${ownerId}`;
        groupName = 'Channel Points Emotes';
      } else {
        groupKey = type;
        groupName = type.charAt(0).toUpperCase() + type.slice(1);
      }
      if (!groups.has(groupKey)) groups.set(groupKey, { name: groupName, emotes: [] });
      groups.get(groupKey)!.emotes.push(emote);
    }
    const sortedGroups = new Map<string, { name: string; emotes: Emote[] }>();
    const keys = Array.from(groups.keys()).sort((a, b) => {
      if (channelId) {
        const aCur = a === `sub-${channelId}`;
        const bCur = b === `sub-${channelId}`;
        if (aCur && !bCur) return -1;
        if (!aCur && bCur) return 1;
      }
      if (a === 'globals') return -1;
      if (b === 'globals') return 1;
      if (a.startsWith('points-') && !b.startsWith('points-')) return -1;
      if (!a.startsWith('points-') && b.startsWith('points-')) return 1;
      if (a.startsWith('sub-') && !b.startsWith('sub-')) return -1;
      if (!a.startsWith('sub-') && b.startsWith('sub-')) return 1;
      const nameA = groups.get(a)?.name || a;
      const nameB = groups.get(b)?.name || b;
      return nameA.localeCompare(nameB);
    });
    for (const key of keys) sortedGroups.set(key, groups.get(key)!);
    return sortedGroups;
  }, [filteredEmotes, channelNameCache, channelId]);

  const groupedKickEmotes = useMemo((): Map<string, { name: string; emotes: Emote[] }> => {
    const groups = new Map<string, { name: string; emotes: Emote[] }>();
    for (const emote of filteredEmotes) {
      const label = emote.emote_type || 'Emotes';
      if (!groups.has(label)) groups.set(label, { name: label, emotes: [] });
      groups.get(label)!.emotes.push(emote);
    }
    return groups;
  }, [filteredEmotes]);

  // Two sections, fixed order: the channel's own emoji first (the reason anyone
  // opens this tab), then YouTube's shared live-chat set. `emote_type` is set
  // when the set is seeded, from YouTube's `isCustomEmoji` flag.
  const groupedYouTubeEmotes = useMemo((): Map<string, { name: string; emotes: Emote[] }> => {
    const custom: Emote[] = [];
    const global: Emote[] = [];
    for (const emote of filteredEmotes) {
      (emote.emote_type === 'youtube' ? global : custom).push(emote);
    }
    const groups = new Map<string, { name: string; emotes: Emote[] }>();
    if (custom.length) groups.set('custom', { name: 'Channel emoji', emotes: custom });
    if (global.length) groups.set('youtube', { name: 'YouTube emoji', emotes: global });
    return groups;
  }, [filteredEmotes]);

  const filteredEmojis = useMemo(() => {
    if (!searchQuery) return allEmojis;
    const query = searchQuery.toLowerCase();
    return allEmojis.filter(({ emoji, category }) => {
      if (category.toLowerCase().includes(query)) return true;
      const keywords = emojiData?.EMOJI_KEYWORDS[emoji];
      return keywords ? keywords.some((k) => k.includes(query)) : false;
    });
  }, [searchQuery, allEmojis]);

  const toggleFavorite = useCallback(
    async (emote: Emote, isFavorited: boolean) => {
      try {
        if (isFavorited) {
          await removeFavoriteEmote(emote.id);
          setFavoriteEmotes((prev) => prev.filter((e) => e.id !== emote.id));
          useAppStore.getState().addToast(`Removed ${emote.name} from favorites`, 'info');
        } else {
          await addFavoriteEmote(emote);
          if (emotes) {
            const all = [...emotes.twitch, ...emotes.bttv, ...emotes['7tv'], ...emotes.ffz, ...emotes.kick, ...emotes.youtube];
            setFavoriteEmotes(getAvailableFavorites(all));
          }
          useAppStore.getState().addToast(`Added ${emote.name} to favorites`, 'success');
        }
      } catch (err) {
        Logger.error('Failed to toggle favorite:', err);
        useAppStore.getState().addToast('Failed to update favorites', 'error');
      }
    },
    [emotes],
  );

  if (!mounted) return null;

  // Each source's tab wears ITS OWN brand colour when active, rather than one
  // green for all of them. Green happens to read as Kick, which made the YouTube
  // tab look like the wrong platform. Sources with no brand of their own (the
  // favourites star, emoji) keep the accent green.
  const TAB_ACCENT: Record<string, string> = {
    twitch: PROVIDERS.twitch.color,
    kick: PROVIDERS.kick.color,
    youtube: PROVIDERS.youtube.color,
    '7tv': '#29b6f6',
    bttv: '#d50014',
    ffz: '#ffffff',
    // GIPHY's brand green, so the tab reads as the GIF source at a glance.
    gifs: '#00ff99',
  };
  const tabClass = (active: boolean) =>
    `flex-1 py-1.5 text-xs transition-all flex items-center justify-center gap-1 ${active ? 'glass-button-active font-extrabold' : 'glass-button text-textSecondary hover:text-white'}`;
  /** Inline colour for an active tab; undefined leaves the class-driven default. */
  const tabStyle = (active: boolean, key?: string): React.CSSProperties => ({
    borderRadius: '8px',
    ...(active && key && TAB_ACCENT[key] ? { color: TAB_ACCENT[key] } : {}),
  });

  return (
    <motion.div
      initial="closed"
      variants={{ open: { opacity: 1, y: 0, scale: 1 }, closed: { opacity: 0, y: 10, scale: 0.98 } }}
      animate={open ? 'open' : 'closed'}
      transition={{ type: 'spring', stiffness: 400, damping: 30 }}
      onAnimationComplete={(def) => {
        if (def === 'closed') setFullyClosed(true);
      }}
      className={className || DEFAULT_PANEL_CLASS}
      style={{
        backgroundColor: 'color-mix(in srgb, var(--color-background) 95%, transparent)',
        display: !open && fullyClosed ? 'none' : undefined,
        pointerEvents: open ? 'auto' : 'none',
      }}
    >
      <div className="p-2 border-b border-borderSubtle">
        <div className="flex items-center gap-1.5">
          <input
            type="text"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder={selectedProvider === 'gifs' ? 'Search GIFs...' : 'Search emotes...'}
            className="flex-1 min-w-0 glass-input text-xs px-3 py-1.5 placeholder-textSecondary"
          />
          {onManageEmotes && (
            <Tooltip content="Manage 7TV emotes" side="top">
              <button
                onClick={() => {
                  onClose();
                  onManageEmotes();
                }}
                className="shrink-0 glass-button p-1.5 text-textSecondary hover:text-white transition-colors"
                style={{ borderRadius: '8px' }}
              >
                <Settings size={15} />
              </button>
            </Tooltip>
          )}
        </div>
        <div className="flex gap-1 mt-2">
          <Tooltip content={`Favorites (${favoriteEmotes.length})`} side="top">
            <button onClick={() => setSelectedProvider('favorites')} className={tabClass(selectedProvider === 'favorites')} style={{ borderRadius: '8px' }}>
              <span className="text-yellow-400">★</span>
              <span className="text-[10px] opacity-70">{favoriteEmotes.length}</span>
            </button>
          </Tooltip>
          <Tooltip content="Emoji" side="top">
            <button onClick={() => setSelectedProvider('emoji')} className={tabClass(selectedProvider === 'emoji')} style={{ borderRadius: '8px' }}>
              <img src={getAppleEmojiUrl('😀')} alt="😀" className="w-4 h-4" />
            </button>
          </Tooltip>
          {/* Shown whenever Twitch ANSWERED the eligibility query, not only when
              the answer is yes. Hiding it on a no made "this channel has GIFs
              off" and "the feature is broken" look identical, with no tab, no
              error and nothing to check. The pane below names which it is. A
              failed query (signed out, offline) still hides the tab, because
              then we genuinely do not know. */}
          {gifStatus && (
            <Tooltip
              content={
                gifStatus.can_use
                  ? 'GIFs'
                  : !gifStatus.is_enabled
                    ? 'GIFs (turned off in this channel)'
                    : 'GIFs (Tier 2 or Tier 3 subscribers)'
              }
              side="top"
            >
              <button onClick={() => setSelectedProvider('gifs')} className={tabClass(selectedProvider === 'gifs')} style={tabStyle(selectedProvider === 'gifs', 'gifs')}>
                <span className="text-[10px] font-extrabold tracking-wide">GIF</span>
              </button>
            </Tooltip>
          )}
          {isTwitch && (
            <Tooltip content={`Twitch (${emotes?.twitch.length || 0})`} side="top">
              <button onClick={() => setSelectedProvider('twitch')} className={tabClass(selectedProvider === 'twitch')} style={tabStyle(selectedProvider === 'twitch', 'twitch')}>
                <EmoteProviderLogo provider="twitch" className="w-4 h-4" />
                <span className="text-[10px] opacity-70">{emotes?.twitch.length || 0}</span>
              </button>
            </Tooltip>
          )}
          {isTwitch && (
            <Tooltip content={`BetterTTV (${emotes?.bttv.length || 0})`} side="top">
              <button onClick={() => setSelectedProvider('bttv')} className={tabClass(selectedProvider === 'bttv')} style={tabStyle(selectedProvider === 'bttv', 'bttv')}>
                <EmoteProviderLogo provider="bttv" className="w-4 h-4" />
                <span className="text-[10px] opacity-70">{emotes?.bttv.length || 0}</span>
              </button>
            </Tooltip>
          )}
          {isKick && (
            <Tooltip content={`Kick (${emotes?.kick.length || 0})`} side="top">
              <button onClick={() => setSelectedProvider('kick')} className={tabClass(selectedProvider === 'kick')} style={tabStyle(selectedProvider === 'kick', 'kick')}>
                <EmoteProviderLogo provider="kick" className="w-4 h-4" />
                <span className="text-[10px] opacity-70">{emotes?.kick.length || 0}</span>
              </button>
            </Tooltip>
          )}
          {isYouTube && (
            <Tooltip
              content={
                emotes?.youtube.length
                  ? `YouTube (${emotes.youtube.length})`
                  : 'YouTube (sign in to load this channel’s emoji)'
              }
              side="top"
            >
              <button onClick={() => setSelectedProvider('youtube')} className={tabClass(selectedProvider === 'youtube')} style={tabStyle(selectedProvider === 'youtube', 'youtube')}>
                <EmoteProviderLogo provider="youtube" className="w-4 h-4" />
                <span className="text-[10px] opacity-70">{emotes?.youtube.length || 0}</span>
              </button>
            </Tooltip>
          )}
          {/* 7TV supports all three platforms natively, so this tab is not gated to
              Twitch and Kick. A channel whose community does not use 7TV simply
              shows zero, the same as anywhere else. */}
          {(isTwitch || isKick || isYouTube) && (
            <Tooltip content={`7TV (${emotes?.['7tv'].length || 0})`} side="top">
              <button onClick={() => setSelectedProvider('7tv')} className={tabClass(selectedProvider === '7tv')} style={tabStyle(selectedProvider === '7tv', '7tv')}>
                <EmoteProviderLogo provider="7tv" className="w-4 h-4" />
                <span className="text-[10px] opacity-70">{emotes?.['7tv'].length || 0}</span>
              </button>
            </Tooltip>
          )}
          {isTwitch && (
            <Tooltip content={`FrankerFaceZ (${emotes?.ffz.length || 0})`} side="top">
              <button onClick={() => setSelectedProvider('ffz')} className={tabClass(selectedProvider === 'ffz')} style={tabStyle(selectedProvider === 'ffz', 'ffz')}>
                <EmoteProviderLogo provider="ffz" className="w-4 h-4" />
                <span className="text-[10px] opacity-70">{emotes?.ffz.length || 0}</span>
              </button>
            </Tooltip>
          )}
        </div>
      </div>
      <div ref={scrollRef} className="flex-1 overflow-y-auto px-2 pb-2 scrollbar-thin">
        {selectedProvider === 'gifs' ? (
          !gifStatus?.can_use ? (
            // Three distinct answers, so an empty pane is never ambiguous:
            // the channel turned GIFs off, or you are not eligible to send here.
            <div className="flex flex-col items-center justify-center h-40 px-6 gap-1.5 text-center">
              <p className="text-xs text-textSecondary leading-relaxed">
                {gifStatus && !gifStatus.is_enabled
                  ? 'This channel has GIFs turned off.'
                  : 'Tier 2 and Tier 3 subscribers can post GIFs in this channel.'}
              </p>
              <p className="text-[11px] text-textSecondary opacity-70 leading-relaxed">
                GIFs other people post still show in your chat.
              </p>
            </div>
          ) : (
            <div className="flex flex-col pt-2">
              {gifNotice && (
                <p className="text-[11px] text-warning px-1 pb-2 leading-relaxed">{gifNotice}</p>
              )}
              {!gifs ? (
                <div className="flex items-center justify-center h-32"><p className="text-xs text-textSecondary">Loading GIFs...</p></div>
              ) : gifs.length === 0 ? (
                <div className="flex items-center justify-center h-32"><p className="text-xs text-textSecondary">No GIFs found</p></div>
              ) : (
                // Two columns: GIPHY art is landscape, so a 7-wide emote grid
                // would render them postage-stamp sized. Fixed row height keeps
                // the scroll position stable while previews stream in.
                <div className="grid grid-cols-2 gap-2 px-1">
                  {gifs.map((gif) => (
                    <button
                      key={gif.id}
                      onClick={() => void onPickGif(gif)}
                      disabled={!!sendingGifId}
                      title={gif.title || 'GIF'}
                      className={`relative overflow-hidden rounded-md border border-borderSubtle bg-black/20 h-24 transition-opacity hover:border-white/25 ${sendingGifId && sendingGifId !== gif.id ? 'opacity-40' : ''}`}
                    >
                      <img
                        src={gif.preview_url}
                        alt={gif.title || 'GIF'}
                        loading="lazy"
                        decoding="async"
                        referrerPolicy="no-referrer"
                        className="h-full w-full object-cover"
                      />
                      {sendingGifId === gif.id && (
                        <span className="absolute inset-0 flex items-center justify-center bg-black/60 text-[11px] font-semibold">
                          Sending...
                        </span>
                      )}
                    </button>
                  ))}
                </div>
              )}
              <p className="text-[10px] text-textSecondary opacity-60 text-center pt-3 pb-1">
                Powered by GIPHY. Picking a GIF posts it straight to chat.
              </p>
            </div>
          )
        ) : selectedProvider === 'emoji' ? (
          filteredEmojis.length === 0 ? (
            <div className="flex items-center justify-center h-32"><p className="text-xs text-textSecondary">No emojis found</p></div>
          ) : (
            <div className="flex flex-col gap-4 pt-2">
              {Object.entries(emojiData?.EMOJI_CATEGORIES ?? {}).map(([category, emojis]) => {
                const filteredCategoryEmojis = searchQuery
                  ? emojis.filter((emoji) => emoji.includes(searchQuery) || category.toLowerCase().includes(searchQuery.toLowerCase()))
                  : emojis;
                if (filteredCategoryEmojis.length === 0) return null;
                return (
                  <div key={category} className="flex flex-col">
                    <h3 className="text-[10px] text-textSecondary uppercase tracking-wider font-bold mb-2 -mx-2 px-4 sticky top-0 py-1.5 border-b border-white/[0.03] z-10 backdrop-blur-ultra" style={{ backgroundColor: 'color-mix(in srgb, var(--color-background) 95%, transparent)' }}>{category}</h3>
                    <div className="grid grid-cols-8 gap-1 px-1">
                      {filteredCategoryEmojis.map((emoji, idx) => (
                        <Tooltip key={`${category}-${idx}`} content={emoji}>
                          <button onClick={() => onInsert(emoji)} className="flex items-center justify-center p-1.5 hover:bg-glass rounded transition-colors">
                            <img
                              src={getAppleEmojiUrl(emoji)}
                              alt={emoji}
                              className="w-6 h-6 object-contain"
                              onError={(e) => {
                                const t = e.currentTarget;
                                if (!t.dataset.fe0f && t.src.endsWith('.png') && !t.src.includes('-fe0f')) {
                                  t.dataset.fe0f = '1';
                                  t.src = t.src.replace(/\.png$/, '-fe0f.png');
                                  return;
                                }
                                t.style.display = 'none';
                                if (t.nextSibling?.textContent !== emoji) t.insertAdjacentText('afterend', emoji);
                              }}
                            />
                          </button>
                        </Tooltip>
                      ))}
                    </div>
                  </div>
                );
              })}
            </div>
          )
        ) : isLoadingEmotes ? (
          <div className="flex items-center justify-center h-32"><p className="text-xs text-textSecondary">Loading emotes...</p></div>
        ) : filteredEmotes.length === 0 ? (
          <div className="flex items-center justify-center h-32 px-6">
            <p className="text-xs text-textSecondary text-center leading-relaxed">
              {selectedProvider === 'youtube' && !searchQuery
                ? // Say why it is empty rather than letting it read as broken.
                  // The full set comes off the live_chat page, which YouTube only
                  // serves to a signed-in viewer (its picker is an authoring
                  // feature). Signed out, the list can still fill in from emoji
                  // actually posted in chat, so both halves are worth saying.
                  'Connect your YouTube account to see this channel’s emoji. Until then, only emoji posted in chat appear here.'
                : 'No emotes found'}
            </p>
          </div>
        ) : selectedProvider === 'twitch' || selectedProvider === 'kick' || selectedProvider === 'youtube' ? (
          <div className="flex flex-col gap-4 pt-2">
            {Array.from((selectedProvider === 'kick' ? groupedKickEmotes : selectedProvider === 'youtube' ? groupedYouTubeEmotes : groupedTwitchEmotes).entries()).map(([groupKey, group]) => (
              <div key={groupKey} className="flex flex-col">
                <h3 className="text-[10px] text-textSecondary uppercase tracking-wider font-bold mb-2 -mx-2 px-4 sticky top-0 py-1.5 border-b border-borderSubtle z-10 backdrop-blur-ultra" style={{ backgroundColor: 'color-mix(in srgb, var(--color-background) 95%, transparent)' }}>
                  <span className="text-textPrimary">{group.name}</span> <span className="opacity-50">({group.emotes.length})</span>
                </h3>
                {chunkArray(group.emotes, TWITCH_COLS * TWITCH_BLOCK_ROWS).map((block, bi) => {
                  const rows = Math.ceil(block.length / TWITCH_COLS);
                  return (
                    <LazyEmoteBlock
                      key={`${groupKey}-blk-${bi}`}
                      scrollRef={scrollRef}
                      estimatedHeight={rows * TWITCH_ROW_PX}
                      gridClass="grid grid-cols-7 gap-2 px-1"
                      onActivate={() => {
                        const tier = inlineEmoteTier();
                        for (const e of block) queueEmoteForDisplayCaching(e.id, e.provider, e.url, tier, true);
                      }}
                    >
                      {() =>
                        block.map((emote, idx) => {
                          const isFavorited = isFavoriteEmote(emote.id);
                          const liveSrc = getCachedEmoteUrl(emote.id, emote.provider) || emote.localUrl || emote.url;
                          return (
                            <div key={`${groupKey}-${emote.provider}-${emote.id}-${idx}`} className="relative group">
                              <Tooltip content={emote.name}>
                                <button onClick={() => onInsert(emote.insertText ?? emote.name)} className="flex flex-col items-center gap-1 p-1.5 hover:bg-glass rounded transition-colors w-full">
                                  <img
                                    src={liveSrc}
                                    alt={emote.name}
                                    loading="lazy"
                                    decoding="async"
                                    referrerPolicy="no-referrer"
                                    crossOrigin="anonymous"
                                    className="w-8 h-8 object-contain"
                                    onError={(e) => {
                                      const target = e.currentTarget;
                                      if (target.src !== emote.url) target.src = emote.url;
                                      else target.style.display = 'none';
                                    }}
                                  />
                                  {!IS_MOBILE && (
                                    <span className="text-xs text-textSecondary truncate w-full text-center">{emote.name}</span>
                                  )}
                                </button>
                              </Tooltip>
                              <Tooltip content={isFavorited ? 'Remove from favorites' : 'Add to favorites'}>
                                <button
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    void toggleFavorite(emote, isFavorited);
                                  }}
                                  className={`absolute top-0 right-0 p-1 rounded-bl transition-all ${isFavorited ? 'text-yellow-400 opacity-100' : 'text-textSecondary opacity-0 group-hover:opacity-100'} hover:text-yellow-400 hover:bg-glass`}
                                >
                                  <svg className="w-3 h-3" fill={isFavorited ? 'currentColor' : 'none'} stroke="currentColor" strokeWidth={2} viewBox="0 0 20 20"><path d="M9.049 2.927c.3-.921 1.603-.921 1.902 0l1.07 3.292a1 1 0 00.95.69h3.462c.969 0 1.371 1.24.588 1.81l-2.8 2.034a1 1 0 00-.364 1.118l1.07 3.292c.3.921-.755 1.688-1.54 1.118l-2.8-2.034a1 1 0 00-1.175 0l-2.8 2.034c-.784.57-1.838-.197-1.539-1.118l1.07-3.292a1 1 0 00-.364-1.118L2.98 8.72c-.783-.57-.38-1.81.588-1.81h3.461a1 1 0 00.951-.69l1.07-3.292z" /></svg>
                                </button>
                              </Tooltip>
                            </div>
                          );
                        })
                      }
                    </LazyEmoteBlock>
                  );
                })}
              </div>
            ))}
          </div>
        ) : (
          <div className="flex flex-col gap-4 pt-2">
            {Array.from(groupedWidthEmotes.values())
              .filter((g) => g.emotes.length > 0)
              .map((group) => (
                <div key={group.label} className="flex flex-col">
                  <h3 className="text-[10px] text-textSecondary uppercase tracking-wider font-bold mb-2 -mx-2 px-4 sticky top-0 py-1.5 border-b border-borderSubtle z-10 backdrop-blur-ultra" style={{ backgroundColor: 'color-mix(in srgb, var(--color-background) 95%, transparent)' }}>
                    <span className="text-textPrimary">{group.label}</span> <span className="opacity-50">({group.emotes.length})</span>
                  </h3>
                  {chunkArray(group.emotes, group.cols * WIDTH_BLOCK_ROWS).map((block, bi) => {
                    const rows = Math.ceil(block.length / group.cols);
                    return (
                      <LazyEmoteBlock
                        key={`${group.label}-blk-${bi}`}
                        scrollRef={scrollRef}
                        estimatedHeight={rows * WIDTH_ROW_PX}
                        gridClass={`grid ${group.gridCols} gap-2 px-1`}
                        onActivate={() => {
                          const tier = inlineEmoteTier();
                          for (const e of block) queueEmoteForDisplayCaching(e.id, e.provider, e.url, tier, true);
                        }}
                      >
                        {() =>
                          block.map((emote: Emote, idx: number) => {
                            const isFavorited = isFavoriteEmote(emote.id);
                            return (
                              <EmoteGridItem
                                key={`${emote.provider}-${emote.id}-${idx}`}
                                emote={emote}
                                isFavorited={isFavorited}
                                onInsert={() => onInsert(emote.insertText ?? emote.name)}
                                onToggleFavorite={() => void toggleFavorite(emote, isFavorited)}
                              />
                            );
                          })
                        }
                      </LazyEmoteBlock>
                    );
                  })}
                </div>
              ))}
          </div>
        )}
      </div>
    </motion.div>
  );
}
