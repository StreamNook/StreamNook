// The chat box's insert menu, lifted out of ChatWidget so every composer
// renders the SAME menu (provider tabs, favorites, emoji, lazy-mounted grids)
// plus the swapping-smiley trigger. The host owns the open state and provides
// a `relative` ancestor for the popover to anchor to.
//
// Everything that goes INTO a message lives here, as sections picked from a
// switcher along the BOTTOM, right above the message box and the smiley that
// opened it (where phone keyboards keep theirs): Emotes (the source tabs),
// GIFs (Twitch, once Twitch says whether this channel allows them) and
// Commands (the browsable list, where the host passes one). Bits is not here:
// spending lives in the channel wallet (chat/ChannelWallet). A host with only
// Emotes gets no switcher at all. The menu grows upward, so a short window
// shrinks it from the top (useFitAbove) and the switcher never gets cut off.

import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode, RefObject } from 'react';
import { motion } from 'framer-motion';
import { Clock, ExternalLink, Lock, Settings, Smile, SquareSlash, Star } from 'lucide-react';
import { Tooltip } from '../ui/Tooltip';
import EmoteProviderLogo from './EmoteProviderLogo';
import { invoke } from '@tauri-apps/api/core';
import {
  type Emote,
  type EmoteSet,
  enhanceRustEmotes,
  getCachedEmoteUrl,
  queueEmoteForDisplayCaching,
  setEmoteCacheBurst,
  inlineEmoteTier,
  sevenTvTierUrl,
} from '../../services/emoteService';
import {
  loadFavoriteEmotes,
  isFavoriteEmote,
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
  matchEmotes,
  personalEmoteOverrides,
  type EmoteMatchChannel,
  type EmoteMatchResult,
  type EmoteMatchRow,
  type PersonalOverride,
} from '../../services/emoteMatch';
import { EmoteCreditLine } from './EmoteCreditLine';
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
import { SearchField } from '../../mobile/ui/SearchField';
import { CommandBrowser, type CommandBrowserProps } from './CommandMenu';
import { useFitAbove } from '../../hooks/useFitAbove';
import { useOutsidePress } from '../../hooks/useOutsidePress';
import { MenuSwitcher, type MenuSwitcherTab } from './MenuSwitcher';
import { staticEmoteUrl } from '../../utils/staticEmoteUrl';
import { fitAmount } from '../../utils/fitNumber';
import { fitHeaderDensity } from './fitHeaderDensity';

type ProviderTab = 'twitch' | 'bttv' | '7tv' | 'ffz' | 'favorites' | 'emoji' | 'kick' | 'youtube' | 'gifs';
/** While searching, the tabs filter the results: `all` is every source. */
type SearchScope = 'all' | ProviderTab;
type EmoteSource = Emote['provider'];
const EMOTE_SOURCES: EmoteSource[] = ['twitch', '7tv', 'bttv', 'ffz', 'kick', 'youtube'];

/** The most rows one search shows; Rust ranks everything and returns these. */
const SEARCH_LIMIT = 200;
/** Emoji shown under the emotes in the All results (the Emoji tab shows all). */
const SEARCH_EMOJI_IN_ALL = 24;

/** A ranked search result with its Emote: the one the menu already holds, or
 *  one built from the row (a personal 7TV emote is in no channel set). */
function rowToEmote(row: EmoteMatchRow, held: Map<string, Emote>): Emote {
  const known = held.get(`${row.provider}:${row.id}`);
  if (known) return known;
  return {
    id: row.id,
    name: row.name,
    url: row.url,
    provider: row.provider as EmoteSource,
    isZeroWidth: row.isZeroWidth,
    localUrl: row.localUrl,
    emote_type: row.emoteType,
    owner_name: row.ownerName,
    original_name: row.originalName,
    width: row.width,
    modifierFlags: row.modifierFlags,
    animated: row.animated,
    insertText: row.insertText,
    ffzSubOnly: row.locked || undefined,
  };
}

/** The menu's own search over the sets it holds, for a host Rust has no
 *  channel for (the mod room) or while the channel's sets are still loading. */
function localSearch(emotes: EmoteSet | null, query: string): EmoteMatchResult {
  const q = query.toLowerCase();
  const rows: EmoteMatchRow[] = [];
  const counts: EmoteMatchResult['counts'] = {};
  let total = 0;
  if (emotes) {
    for (const source of EMOTE_SOURCES) {
      for (const e of emotes[source] ?? []) {
        if (!e.name.toLowerCase().includes(q)) continue;
        total++;
        counts[source] = (counts[source] ?? 0) + 1;
        if (rows.length < SEARCH_LIMIT) {
          rows.push({ id: e.id, name: e.name, url: e.url, provider: source, sourceLabel: '', sourceDetail: '' });
        }
      }
    }
  }
  return { rows, total, ready: true, counts };
}

/** Grid columns a result takes: wide 7TV emotes span two or three cells. */
function widthSpan(e: Emote): string {
  const w = e.width || 32;
  return w <= 48 ? '' : w <= 80 ? 'col-span-2' : 'col-span-3';
}

/** A bigger image for a hover card where the provider has one. */
function hoverImageUrl(emote: Emote): string {
  if (emote.provider === '7tv') return `https://cdn.7tv.app/emote/${emote.id}/4x.avif`;
  if (emote.provider === 'twitch') return emote.url.replace(/\/1\.0$/, '/3.0');
  return emote.localUrl || emote.url;
}

/** What leads a section header on the phone: the tab's provider logo, a clock
 *  on Recently used, a star on the rest of Favorites. The logo already names
 *  the provider, so the header keeps only the section's own title beside it.
 *  FrankerFaceZ's brand is white, so it takes the text color and reads in
 *  both themes. */
function SectionMark({ tab, sectionKey }: { tab: ProviderTab; sectionKey: string }) {
  if (tab === 'favorites') {
    return sectionKey === 'recent' ? (
      <Clock className="w-[11px] h-[11px] shrink-0" strokeWidth={2.75} aria-hidden="true" />
    ) : (
      <Star className="w-[11px] h-[11px] shrink-0 text-yellow-400" fill="currentColor" strokeWidth={0} aria-hidden="true" />
    );
  }
  if (tab === 'emoji' || tab === 'gifs') return null;
  return <EmoteProviderLogo provider={tab} tinted={tab !== 'ffz'} className="w-[11px] h-[11px] shrink-0" />;
}

/** The viewer's personal emote that shows instead of a channel emote of the
 *  same name in their own messages. */
interface ReplacedBy {
  id: string;
  url: string;
}

/** One picker cell as Rust sends it (`emote_picker.rs`), shaped for the page. */
type PickerCell = Emote & { favorite?: boolean; replaced_by?: ReplacedBy };

/** A section of the selected tab, in the order Rust put it. */
interface PickerGroup {
  key: string;
  title: string;
  /** Grid columns (7, or 4 and 3 for wide and ultra wide emotes). */
  columns: number;
  emotes: PickerCell[];
}

/** Rust's answer for the open picker. */
interface PickerAnswer {
  ready: boolean;
  tabs: { id: string; count: number }[];
  tab: string;
  sections: { key: string; title: string; columns: number; emotes: unknown[] }[];
}

const GRID_COLS: Record<number, string> = { 3: 'grid-cols-3', 4: 'grid-cols-4', 7: 'grid-cols-7' };

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
/** Emotes found to have no first-frame file (`provider:id`). The CDNs publish
 *  one only for emotes that animate, so an emote whose animation is unknown
 *  answers 404 for it; once seen, it is drawn live and never asked again. */
const noStillFrame = new Set<string>();

const EmoteGridItem = memo(
  ({
    emote,
    isFavorited,
    still,
    replacedBy,
    onInsert,
    onToggleFavorite,
    channelLogin,
    channelId,
  }: {
    emote: Emote;
    isFavorited: boolean;
    /** Draw the first frame (see `LazyEmoteBlock`); hovering still animates it. */
    still: boolean;
    /** Your personal emote that shows instead when you type this name. */
    replacedBy?: ReplacedBy;
    onInsert: () => void;
    onToggleFavorite: () => void;
    /** The room the menu is open for, so the credit can come from its sets. */
    channelLogin?: string;
    channelId?: string;
  }) => {
    const [hovered, setHovered] = useState(false);
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
    // An emote known to be still has no frames to save: it keeps its disk copy.
    const stillKey = `${emote.provider}:${emote.id}`;
    const showStill = still && !hovered && emote.animated !== false && !noStillFrame.has(stillKey);
    const liveSrc = is7tv
      ? liveLocal || emote.localUrl || sevenTvTierUrl(emote.id, emoteTier)
      : liveLocal || emote.localUrl || emote.url;
    const gridSrc = showStill
      ? staticEmoteUrl(is7tv ? sevenTvTierUrl(emote.id, emoteTier) : emote.url)
      : liveSrc;
    const hoverPreviewSize = useAppStore((s) => s.settings.chat_design?.emote_hover_size) ?? 96;

    return (
      <Tooltip
        side="top"
        delay={200}
        content={
          <div className="flex flex-col items-center gap-1.5 py-0.5">
            <img
              src={hoverImageUrl(emote)}
              alt={emote.name}
              className="w-auto object-contain mx-auto drop-shadow-md"
              style={{ height: hoverPreviewSize, maxWidth: hoverPreviewSize * 2, ...staticModifierStyle(emote.modifierFlags) }}
              onError={(e) => {
                const t = e.currentTarget;
                if (emote.provider === 'twitch' && t.src !== emote.url) {
                  t.src = emote.url;
                  return;
                }
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
              <EmoteCreditLine
                provider={emote.provider}
                id={emote.id}
                name={emote.name}
                ownerHint={emote.owner_name}
                originalHint={emote.original_name}
                channelLogin={channelLogin}
                channelId={channelId}
              />
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
              {replacedBy && (
                <span className="flex items-center gap-1 mt-1 text-[10px] leading-tight text-white/70">
                  <img src={replacedBy.url} alt="" className="h-4 w-auto object-contain" />
                  Your personal {emote.name} shows instead
                </span>
              )}
            </div>
          </div>
        }
      >
        <div
          className="relative group flex items-center justify-center focus:outline-none w-full h-full min-h-8"
          style={{ contentVisibility: 'auto', containIntrinsicBlockSize: '40px' }}
          onPointerEnter={still ? () => setHovered(true) : undefined}
          onPointerLeave={still ? () => setHovered(false) : undefined}
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
            } ${lockedSub ? 'opacity-40 cursor-not-allowed' : replacedBy ? 'opacity-50' : ''}`}
          >
            <img
              src={gridSrc}
              srcSet={is7tv && !emote.localUrl && !showStill ? `https://cdn.7tv.app/emote/${emote.id}/1x.avif 1x, https://cdn.7tv.app/emote/${emote.id}/2x.avif 2x` : undefined}
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
              onLoad={(e) => {
                // A good load ends any fallback run, so failures from earlier
                // scrolls never add up to a faded emote.
                const t = e.currentTarget;
                delete t.dataset.fb;
                t.style.opacity = '';
              }}
              onError={(e) => {
                const t = e.currentTarget;
                if (showStill && t.src !== liveSrc) {
                  // No first frame published: the emote does not animate.
                  noStillFrame.add(stillKey);
                  t.src = liveSrc;
                  return;
                }
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
// Emoji grid: 8 across, a 24px image in a 6px-padded button, rows 4px apart.
const EMOJI_COLS = 8;
const EMOJI_BLOCK_ROWS = 6;
const EMOJI_ROW_PX = 40;

const LazyEmoteBlock = memo(
  ({
    scrollRef,
    estimatedHeight,
    gridClass,
    animate,
    onActivate,
    children,
  }: {
    scrollRef: RefObject<HTMLDivElement | null>;
    estimatedHeight: number;
    gridClass: string;
    /** Animation is allowed right now: the list is at rest and the viewer's
     *  setting permits it. Only blocks actually on screen then animate. */
    animate: boolean;
    onActivate?: () => void;
    /** `still` = draw first frames. */
    children: (still: boolean) => ReactNode;
  }) => {
    const ref = useRef<HTMLDivElement>(null);
    const [visible, setVisible] = useState(false);
    const [onScreen, setOnScreen] = useState(false);
    const activatedRef = useRef(false);
    // Mounted blocks include 600px of overscan each way, and a fast scroll
    // mounts a great many on the way past. Animated emotes cost their decoded
    // frames (megabytes each for a long 7TV animation), so only what is on
    // screen animates; the rest draw their first frame.
    useEffect(() => {
      const el = ref.current;
      const root = scrollRef.current;
      if (!el || !root) return;
      const obs = new IntersectionObserver((entries) => setOnScreen(entries[0]?.isIntersecting ?? false), { root });
      obs.observe(el);
      return () => obs.disconnect();
    }, [scrollRef]);
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
        {visible ? children(!(animate && onScreen)) : null}
      </div>
    );
  },
);

// ── the panel ────────────────────────────────────────────────────────────────
export interface EmotePickerPanelProps {
  open: boolean;
  onClose: () => void;
  /** The button that opens it: pressing it is its own toggle, not an outside press. */
  ignoreRef?: RefObject<HTMLElement | null>;
  emotes: EmoteSet | null;
  isTwitch: boolean;
  isKick: boolean;
  /** YouTube chat: shows the emoji learned from this room's messages. */
  isYouTube?: boolean;
  channelId?: string;
  channelLogin?: string;
  isLoadingEmotes?: boolean;
  onInsert: (text: string) => void;
  onManageEmotes?: () => void;
  /** Moves the menu into a window of its own beside the chat. Absent where
   *  it cannot pop out (a phone, or already popped out). */
  onPopOut?: () => void;
  /** Positioning for the popover (defaults to full-width above the composer). */
  className?: string;
  /** The Commands section. Absent where the host has no command list to
   *  offer (the mod room, the pop-out palette, the phone's own sheet). */
  commands?: Omit<CommandBrowserProps, 'onClose'>;
  /** The GIFs section. Off where there is no channel to post into (the mod
   *  room); on Twitch it still waits for Twitch's eligibility answer. */
  gifs?: boolean;
}

/** The popover's height when the room above the message box allows it. */
const PANEL_MAX_PX = 520;
// The same frosted surface as the Tools menu and the command list's dialog
// (`.sn-popover`: blur plus a tint that rides the Glassiness slider and goes
// solid at 0%), so every menu over the message box reads as one family.
// z-50: above a pinned prediction or poll (z-40), which otherwise covered the
// top of the open menu, where search results land.
const DEFAULT_PANEL_CLASS =
  'sn-popover absolute bottom-full left-0 right-0 mb-2 h-[520px] flex flex-col overflow-hidden origin-bottom z-50';
/** A host that places the menu itself (the phone) keeps the plain fill. */
const HOST_PANEL_FILL =
  'color-mix(in srgb, var(--color-background) calc(95% + (1 - var(--glass-strength, 1)) * 5%), transparent)';

export function EmotePickerPanel({
  open,
  onClose,
  ignoreRef,
  emotes,
  isTwitch,
  isKick,
  isYouTube = false,
  channelId,
  channelLogin,
  isLoadingEmotes = false,
  onInsert,
  onManageEmotes,
  onPopOut,
  className,
  commands,
  gifs: gifsSection = true,
}: EmotePickerPanelProps) {
  const [mounted, setMounted] = useState(open);
  const [fullyClosed, setFullyClosed] = useState(!open);
  // Opening mounts the panel and cancels "fully closed" during render
  // (adjust-state-on-prop-change), so the first open frame already has it.
  const [seenOpen, setSeenOpen] = useState(open);
  // Commands is a detour: every open starts back on the emotes.
  const [detour, setDetour] = useState<'commands' | null>(null);
  if (open !== seenOpen) {
    setSeenOpen(open);
    if (open) {
      setMounted(true);
      setFullyClosed(false);
      setDetour(null);
    }
  }

  const [selectedProvider, setSelectedProvider] = useState<ProviderTab>(
    isTwitch ? 'twitch' : isKick ? 'kick' : isYouTube ? 'youtube' : 'emoji',
  );
  // The emote tab to come back to from GIFs.
  const [lastEmoteTab, setLastEmoteTab] = useState<ProviderTab>(selectedProvider);
  const [searchQuery, setSearchQuery] = useState('');
  // Typing searches every source at once; the tabs then filter the results.
  const [searchScope, setSearchScope] = useState<SearchScope>('all');
  // The source tabs drop their counts (globals.css `.sn-emote-tabs`) when the
  // panel is too narrow for them; the tooltips keep the exact numbers.
  const [tabRowEl, setTabRowEl] = useState<HTMLDivElement | null>(null);
  useEffect(() => (tabRowEl ? fitHeaderDensity(tabRowEl, 1, 0) : undefined), [tabRowEl]);
  // Kept with the query it answers. The last answer stays on screen until the
  // next one lands, so narrowing a search never flickers.
  const [searchResult, setSearchResult] = useState<(EmoteMatchResult & { query: string }) | null>(null);
  const onSearchChange = (next: string) => {
    // A fresh search looks everywhere, except from the Emoji tab, where the
    // person is plainly after an emoji.
    if (!searchQuery.trim() && next.trim()) setSearchScope(selectedProvider === 'emoji' ? 'emoji' : 'all');
    setSearchQuery(next);
  };
  const scrollRef = useRef<HTMLDivElement>(null);
  // Fit the default popover into the room above the message box. A host that
  // positions the menu itself (the pop-out window) sizes it itself.
  const panelRef = useRef<HTMLDivElement>(null);
  useFitAbove(panelRef, open && !className, PANEL_MAX_PX);
  // A press anywhere else closes it, the expanded Commands dialog included as inside.
  const markInside = useOutsidePress(open, [panelRef, ignoreRef], onClose);
  // Emotes animate only while the list is at rest, like the chat's own
  // "Animate emotes" setting allows: during a scroll every cell going past
  // draws its first frame, so a fling never decodes hundreds of animations.
  const animateSetting = useAppStore((s) => s.settings.chat_design?.animate_emotes) ?? 'always';
  const [scrollSettled, setScrollSettled] = useState(true);
  useEffect(() => {
    const el = scrollRef.current;
    if (!el || !mounted) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const onScroll = () => {
      setScrollSettled(false);
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => setScrollSettled(true), 200);
    };
    el.addEventListener('scroll', onScroll, { passive: true });
    return () => {
      el.removeEventListener('scroll', onScroll);
      if (timer) clearTimeout(timer);
    };
  }, [mounted]);
  const animateGrid = animateSetting === 'always' && scrollSettled;
  const hoverPreviewSize = useAppStore((s) => s.settings.chat_design?.emote_hover_size) ?? 96;

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

  const gifStatus = gifsSection && gifStatusFor && gifStatusFor.key === channelId ? gifStatusFor.status : null;
  const gifQueryKey = `${channelId ?? ''}|${searchQuery}`;
  // null means "no results for THIS key yet", which is what renders the
  // loading state; an empty array means the search genuinely returned nothing.
  const gifs = gifResults && gifResults.key === gifQueryKey ? gifResults.items : null;

  // Ask once per channel, and only where GIFs can exist: Twitch, with an id,
  // while the picker is actually open. Nothing runs for a closed picker.
  useEffect(() => {
    if (!open || !isTwitch || !channelId || !gifsSection) return;
    let cancelled = false;
    void getGifPickerStatus(channelId)
      .then((status) => { if (!cancelled) setGifStatusFor({ key: channelId, status }); })
      .catch(() => { if (!cancelled) setGifStatusFor(null); });
    return () => { cancelled = true; };
  }, [open, isTwitch, channelId, gifsSection]);

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

  // The tabs, their counts, the selected tab's sections and every cell's
  // flags (favorite, locked, replaced by your personal emote) come from Rust
  // (`emote_picker`), asked again whenever the tab, the search, the room's set
  // or your favorites change. YouTube emoji are not in Rust's set yet, so the
  // YouTube tab still groups its own below.
  const pickerProvider = isTwitch ? 'twitch' : isKick ? 'kick' : isYouTube ? 'youtube' : null;
  const rustTab = selectedProvider !== 'emoji' && selectedProvider !== 'gifs' && selectedProvider !== 'youtube';
  const trimmedQuery = searchQuery.trim();
  // A typed query searches every source (below), so the tab's own answer is
  // only ever asked for unfiltered.
  const searching = trimmedQuery.length > 0 && selectedProvider !== 'gifs';
  const pickerQuery = rustTab && !searching ? searchQuery : '';
  const [favoritesVersion, setFavoritesVersion] = useState(0);
  const pickerKey = `${pickerProvider}|${channelLogin ?? ''}|${channelId ?? ''}|${rustTab ? selectedProvider : ''}|${pickerQuery}|${favoritesVersion}`;
  const [picker, setPicker] = useState<{ key: string; answer: PickerAnswer } | null>(null);
  useEffect(() => {
    if (!open || !pickerProvider || !channelLogin) return;
    let cancelled = false;
    void invoke<PickerAnswer>('emote_picker', {
      provider: pickerProvider,
      channel: channelLogin,
      channelId: channelId || null,
      tab: rustTab ? selectedProvider : null,
      query: pickerQuery,
      tier: inlineEmoteTier(),
    })
      .then((answer) => {
        if (!cancelled) setPicker({ key: pickerKey, answer });
      })
      .catch((err) => Logger.warn('[EmotePicker] emote_picker failed:', err));
    return () => {
      cancelled = true;
    };
    // `emotes` is a dependency on purpose: a new or changed set means new rows.
  }, [open, pickerProvider, channelLogin, channelId, rustTab, selectedProvider, pickerQuery, pickerKey, emotes]);
  const answer = picker?.answer ?? null;
  const rustTabCount = (id: string) => answer?.tabs.find((t) => t.id === id)?.count ?? 0;

  const matchTarget = useMemo<EmoteMatchChannel | null>(
    () =>
      channelLogin
        ? { provider: isKick ? 'kick' : isYouTube ? 'youtube' : 'twitch', channel: channelLogin, channelId: channelId ?? null }
        : null,
    [channelLogin, channelId, isKick, isYouTube],
  );

  // Rust ranks across every source over the sets it holds (the same matcher
  // as the message box's `:` list). A host it has no channel for, or a channel
  // still loading, falls back to the menu's own search over what it holds.
  useEffect(() => {
    if (!open || !searching) return;
    let alive = true;
    const timer = setTimeout(() => {
      const ask = matchTarget
        ? matchEmotes(matchTarget, trimmedQuery, 'search', { limit: SEARCH_LIMIT, picker: true })
        : Promise.resolve(null);
      void ask.then((res) => {
        if (!alive) return;
        const result = res && res.ready ? res : localSearch(emotes, trimmedQuery);
        setSearchResult({ ...result, query: trimmedQuery });
      });
    }, 40);
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [open, searching, trimmedQuery, matchTarget, emotes]);

  // Channel emotes your own 7TV personal emote of the same name replaces when
  // you type them, so the search results can say so (the tabs' cells carry it
  // from Rust's picker answer). Asked again when the set changes.
  const [overrides, setOverrides] = useState<Map<string, PersonalOverride> | null>(null);
  useEffect(() => {
    if (!open || !searching || !isTwitch || !channelLogin) return;
    let cancelled = false;
    void personalEmoteOverrides(channelLogin, channelId).then((map) => {
      if (!cancelled) setOverrides(map);
    });
    return () => {
      cancelled = true;
    };
  }, [open, searching, isTwitch, channelLogin, channelId, emotes]);

  const heldEmotes = useMemo(() => {
    const map = new Map<string, Emote>();
    if (!emotes) return map;
    for (const source of EMOTE_SOURCES) for (const e of emotes[source] ?? []) map.set(`${source}:${e.id}`, e);
    return map;
  }, [emotes]);

  const searchEmotes = useMemo((): Emote[] => {
    if (!searching || !searchResult) return [];
    const all = searchResult.rows.map((row) => ({ row, emote: rowToEmote(row, heldEmotes) }));
    const scoped =
      searchScope === 'all'
        ? all
        : searchScope === 'favorites'
          ? all.filter(({ emote }) => isFavoriteEmote(emote.id))
          : all.filter(({ row }) => row.provider === searchScope);
    return scoped.map(({ emote }) => emote);
    // `favoritesVersion` re-reads the stars after a toggle.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searching, searchResult, heldEmotes, searchScope, favoritesVersion]);

  // Aggressive disk caching while the picker is open; polite trickle on close.
  useEffect(() => {
    if (!open) return;
    setEmoteCacheBurst(true);
    return () => setEmoteCacheBurst(false);
  }, [open]);

  // The stars read the saved favorites; load them once the picker opens.
  useEffect(() => {
    if (!open) return;
    void loadFavoriteEmotes().then(() => setFavoritesVersion((v) => v + 1));
  }, [open]);

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

  // The emoji tab's sections after the search, computed once and read by both
  // the grid and its empty state, so the two can never disagree (they did: the
  // empty check knew keywords and the grid did not, so "heart" drew a blank
  // pane). A query matches a category name, an emoji's keywords, or the emoji
  // itself when one is pasted in. Categories left empty are dropped.
  const emojiSections = useMemo((): [string, string[]][] => {
    if (!emojiData) return [];
    const query = searchQuery.trim().toLowerCase();
    return Object.entries(emojiData.EMOJI_CATEGORIES)
      .map(([category, emojis]): [string, string[]] =>
        !query || category.toLowerCase().includes(query)
          ? [category, emojis]
          : [
              category,
              emojis.filter(
                (emoji) =>
                  emoji.includes(query) ||
                  (emojiData.EMOJI_KEYWORDS[emoji]?.some((k) => k.includes(query)) ?? false),
              ),
            ],
      )
      .filter(([, emojis]) => emojis.length > 0);
  }, [emojiData, searchQuery]);
  // The same matches as one list, for the cross-source search results.
  const filteredEmojis = useMemo(
    () => emojiSections.flatMap(([category, emojis]) => emojis.map((emoji) => ({ emoji, category }))),
    [emojiSections],
  );

  // The selected tab's sections. Rust's answer is used once it is for this
  // tab; until then (or for YouTube, see below) the groups come from here.
  const rustGroups = useMemo((): PickerGroup[] | null => {
    if (!rustTab || !answer || answer.tab !== selectedProvider) return null;
    const tier = inlineEmoteTier();
    return answer.sections.map((section) => ({
      key: section.key,
      title: section.title,
      columns: section.columns,
      emotes: enhanceRustEmotes(section.emotes, tier) as PickerCell[],
    }));
  }, [rustTab, answer, selectedProvider]);

  const youtubeEmotes = useMemo((): Emote[] => {
    if (selectedProvider !== 'youtube') return [];
    const all = emotes?.youtube ?? [];
    if (!searchQuery) return all;
    const query = searchQuery.toLowerCase();
    return all.filter((e) => e.name.toLowerCase().includes(query));
  }, [selectedProvider, emotes, searchQuery]);

  // Two sections, fixed order: the channel's own emoji first (the reason anyone
  // opens this tab), then YouTube's shared live-chat set. `emote_type` is set
  // when the set is seeded, from YouTube's `isCustomEmoji` flag.
  const groupedYouTubeEmotes = useMemo((): PickerGroup[] => {
    const custom: Emote[] = [];
    const global: Emote[] = [];
    for (const emote of youtubeEmotes) {
      (emote.emote_type === 'youtube' ? global : custom).push(emote);
    }
    const groups: PickerGroup[] = [];
    if (custom.length) groups.push({ key: 'custom', title: 'Channel emoji', columns: 7, emotes: custom });
    if (global.length) groups.push({ key: 'youtube', title: 'YouTube emoji', columns: 7, emotes: global });
    return groups;
  }, [youtubeEmotes]);

  const groups: PickerGroup[] = selectedProvider === 'youtube' ? groupedYouTubeEmotes : (rustGroups ?? []);
  const waitingForRust = rustTab && !rustGroups && !!pickerProvider;
  // Twitch, Kick and YouTube cells keep their names under the image on desktop.
  const namedCells = selectedProvider === 'twitch' || selectedProvider === 'kick' || selectedProvider === 'youtube';

  const toggleFavorite = useCallback(
    async (emote: Emote, isFavorited: boolean) => {
      try {
        if (isFavorited) {
          await removeFavoriteEmote(emote.id);
          useAppStore.getState().addToast(`Removed ${emote.name} from favorites`, 'info');
        } else {
          await addFavoriteEmote(emote);
          useAppStore.getState().addToast(`Added ${emote.name} to favorites`, 'success');
        }
        // Rust lists the Favorites tab from the saved list: ask again.
        setFavoritesVersion((v) => v + 1);
      } catch (err) {
        Logger.error('Failed to toggle favorite:', err);
        useAppStore.getState().addToast('Failed to update favorites', 'error');
      }
    },
    [],
  );

  if (!mounted) return null;

  // Each source's tab wears ITS OWN brand color when active, rather than one
  // green for all of them. Green happens to read as Kick, which made the YouTube
  // tab look like the wrong platform. Sources with no brand of their own (the
  // favorites star, emoji) keep the accent green.
  const TAB_ACCENT: Record<string, string> = {
    twitch: PROVIDERS.twitch.color,
    kick: PROVIDERS.kick.color,
    youtube: PROVIDERS.youtube.color,
    '7tv': '#29b6f6',
    bttv: '#d50014',
    ffz: '#ffffff',
  };
  const tabClass = (active: boolean) =>
    `flex-1 py-1.5 text-xs transition-all flex items-center justify-center gap-1 ${active ? 'glass-button-active font-extrabold' : 'glass-button text-textSecondary hover:text-white'}`;
  /** Inline color for an active tab; undefined leaves the class-driven default. */
  const tabStyle = (active: boolean, key?: string): React.CSSProperties => ({
    borderRadius: '8px',
    ...(active && key && TAB_ACCENT[key] ? { color: TAB_ACCENT[key] } : {}),
  });

  // While searching, the tabs filter the ranked results and count the hits in
  // each source; otherwise they pick a source and count what Rust says it holds.
  const activeTab: SearchScope = searching ? searchScope : selectedProvider;
  const pickTab = (tab: ProviderTab) => (searching ? setSearchScope(tab) : setSelectedProvider(tab));
  const emojiHits = searching ? filteredEmojis.length : 0;
  const tabCount = (tab: ProviderTab | 'all'): number => {
    if (searching) {
      const counts = searchResult?.counts ?? {};
      if (tab === 'all') return (searchResult?.total ?? 0) + emojiHits;
      if (tab === 'favorites') return searchResult?.rows.filter((r) => isFavoriteEmote(r.id)).length ?? 0;
      if (tab === 'emoji') return emojiHits;
      return counts[tab as EmoteSource] ?? 0;
    }
    if (tab === 'youtube') return emotes?.youtube.length || 0;
    if (tab === 'all' || tab === 'emoji' || tab === 'gifs') return 0;
    return rustTabCount(tab);
  };

  type Section = 'emotes' | 'gifs' | 'commands';
  const section: Section =
    detour === 'commands' && commands
      ? 'commands'
      : selectedProvider === 'gifs'
        ? 'gifs'
        : 'emotes';
  const pickSection = (next: Section) => {
    setDetour(next === 'commands' ? next : null);
    if (next === 'gifs' && selectedProvider !== 'gifs') {
      setLastEmoteTab(selectedProvider);
      setSelectedProvider('gifs');
    } else if (next === 'emotes' && selectedProvider === 'gifs') {
      setSelectedProvider(lastEmoteTab === 'gifs' ? 'emoji' : lastEmoteTab);
    }
  };
  // GIFs shows whenever Twitch ANSWERED the eligibility query, not only when
  // the answer is yes. Hiding it on a no made "this channel has GIFs off" and
  // "the feature is broken" look identical, with no tab, no error and nothing
  // to check. The pane below names which it is. A failed query (signed out,
  // offline) still hides it, because then we genuinely do not know.
  const sections: MenuSwitcherTab<Section>[] = [
    { key: 'emotes', label: 'Emotes', icon: <Smile size={15} className="flex-shrink-0" aria-hidden /> },
    ...(gifStatus
      ? [{
          key: 'gifs' as const,
          label: 'GIFs',
          icon: (
            <span className="flex-shrink-0 rounded border border-current px-[3px] text-[8.5px] font-extrabold leading-[12px] tracking-wide" aria-hidden>
              GIF
            </span>
          ),
          // Only GIFs has more to say than its label: whether the channel
          // turned them off or they need Tier 2 or 3.
          hint: gifStatus.can_use
            ? undefined
            : !gifStatus.is_enabled
              ? 'GIFs (turned off in this channel)'
              : 'GIFs (Tier 2 or Tier 3 subscribers)',
        }]
      : []),
    ...(commands
      ? [{ key: 'commands' as const, label: 'Commands', icon: <SquareSlash size={15} className="flex-shrink-0" aria-hidden /> }]
      : []),
  ];

  const sectionHeadingStyle: React.CSSProperties = {
    backgroundColor: 'color-mix(in srgb, var(--color-background) calc(95% + (1 - var(--glass-strength, 1)) * 5%), transparent)',
  };

  const renderEmojiButton = (emoji: string, key: string) => (
    <Tooltip key={key} content={emoji}>
      <button
        onClick={() => onInsert(emoji)}
        // An emoji with no image falls back to its character (onError below).
        // On the phone that text gets the image's 24px box, not the panel's
        // inherited size.
        className={`flex items-center justify-center p-1.5 hover:bg-glass rounded transition-colors${IS_MOBILE ? ' text-[20px] leading-6' : ''}`}
      >
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
  );

  /** One ranked grid across every source, best match first. Wide emotes take
   *  two or three cells and the grid packs around them, so the order holds
   *  without the ragged rows a flat 7-across grid would leave. */
  function renderSearchResults() {
    const showEmotes = searchScope !== 'emoji';
    const showEmoji = searchScope === 'all' || searchScope === 'emoji';
    const emoji = showEmoji
      ? searchScope === 'emoji'
        ? filteredEmojis
        : filteredEmojis.slice(0, SEARCH_EMOJI_IN_ALL)
      : [];
    const scopeTotal = searchScope === 'all' ? (searchResult?.total ?? 0) : tabCount(searchScope);
    const capped = showEmotes && scopeTotal > searchEmotes.length;
    if (!searchResult && showEmotes) {
      return <div className="flex items-center justify-center h-32"><p className="text-xs text-textSecondary">Searching...</p></div>;
    }
    if ((!showEmotes || searchEmotes.length === 0) && emoji.length === 0) {
      return (
        <div className="flex items-center justify-center h-32 px-6">
          <p className="text-xs text-textSecondary text-center leading-relaxed">
            {searchScope === 'emoji' ? 'No emoji found' : `No emotes found for "${trimmedQuery}"`}
          </p>
        </div>
      );
    }
    return (
      <div className="flex flex-col gap-4 pt-2">
        {showEmotes && searchEmotes.length > 0 && (
          <div className="flex flex-col">
            {chunkArray(searchEmotes, TWITCH_COLS * WIDTH_BLOCK_ROWS).map((block, bi) => (
              <LazyEmoteBlock
                key={`search-${searchScope}-blk-${bi}`}
                scrollRef={scrollRef}
                estimatedHeight={Math.ceil(block.length / TWITCH_COLS) * WIDTH_ROW_PX}
                gridClass="grid grid-cols-7 grid-flow-row-dense gap-2 px-1"
                animate={animateGrid}
                onActivate={() => {
                  const tier = inlineEmoteTier();
                  for (const e of block) queueEmoteForDisplayCaching(e.id, e.provider, e.url, tier, true);
                }}
              >
                {(still) =>
                  block.map((emote) => {
                    const isFavorited = isFavoriteEmote(emote.id);
                    return (
                      <div key={`${emote.provider}-${emote.id}`} className={widthSpan(emote)}>
                        <EmoteGridItem
                          emote={emote}
                          isFavorited={isFavorited}
                          still={still}
                          replacedBy={emote.provider !== 'twitch' ? overrides?.get(emote.name) : undefined}
                          onInsert={() => onInsert(emote.insertText ?? emote.name)}
                          onToggleFavorite={() => void toggleFavorite(emote, isFavorited)}
                          channelLogin={channelLogin}
                          channelId={channelId}
                        />
                      </div>
                    );
                  })
                }
              </LazyEmoteBlock>
            ))}
            {capped && (
              <p className="text-[10px] text-textSecondary opacity-70 text-center pt-2">
                Showing the best {searchEmotes.length} of {scopeTotal}. Keep typing to narrow it down.
              </p>
            )}
          </div>
        )}
        {emoji.length > 0 && (
          <div className="flex flex-col">
            {searchScope === 'all' && (
              <h3 className="text-[10px] text-textSecondary uppercase tracking-wider font-bold mb-2 -mx-2 px-4 sticky top-0 py-1.5 border-b border-borderSubtle z-10 backdrop-blur-ultra" style={sectionHeadingStyle}>
                <span className="text-textPrimary">Emoji</span> <span className="opacity-50">({filteredEmojis.length})</span>
              </h3>
            )}
            {chunkArray(emoji, EMOJI_COLS * EMOJI_BLOCK_ROWS).map((block, bi) => (
              <LazyEmoteBlock
                key={`search-emoji-${searchScope}-blk-${bi}`}
                scrollRef={scrollRef}
                estimatedHeight={Math.ceil(block.length / EMOJI_COLS) * EMOJI_ROW_PX}
                gridClass="grid grid-cols-8 gap-1 px-1"
                animate={false}
              >
                {() => block.map(({ emoji: ch }) => renderEmojiButton(ch, `search-emoji-${ch}`))}
              </LazyEmoteBlock>
            ))}
            {searchScope === 'all' && filteredEmojis.length > emoji.length && (
              <button
                onClick={() => setSearchScope('emoji')}
                className="self-center mt-2 text-[10px] text-textSecondary hover:text-white transition-colors"
              >
                Show all {filteredEmojis.length} emoji
              </button>
            )}
          </div>
        )}
      </div>
    );
  }

  return (
    <motion.div
      initial="closed"
      variants={{ open: { opacity: 1, y: 0, scale: 1 }, closed: { opacity: 0, y: 10, scale: 0.98 } }}
      animate={open ? 'open' : 'closed'}
      transition={{ type: 'spring', stiffness: 400, damping: 30 }}
      onAnimationComplete={(def) => {
        if (def === 'closed') setFullyClosed(true);
      }}
      ref={panelRef}
      onPointerDownCapture={markInside}
      className={className || DEFAULT_PANEL_CLASS}
      style={{
        backgroundColor: className ? HOST_PANEL_FILL : undefined,
        display: !open && fullyClosed ? 'none' : undefined,
        pointerEvents: open ? 'auto' : 'none',
      }}
    >
      {section === 'commands' && commands && <CommandBrowser {...commands} onClose={onClose} />}
      {/* Kept mounted (only hidden) under Commands, so the grid's scroll
          listener and lazy blocks survive a trip there and back. */}
      <div
        className="flex min-h-0 flex-1 flex-col"
        style={section === 'commands' ? { display: 'none' } : undefined}
      >
      <div className="p-2 border-b border-borderSubtle">
        <div className="flex items-center gap-1.5">
          {IS_MOBILE ? (
            // The phone's one search field (magnifier, clear button, 44px).
            <SearchField
              value={searchQuery}
              onChange={onSearchChange}
              placeholder={selectedProvider === 'gifs' ? 'Search GIFs…' : 'Search emotes…'}
              className="flex-1 min-w-0"
            />
          ) : (
            <input
              type="text"
              value={searchQuery}
              onChange={(e) => onSearchChange(e.target.value)}
              placeholder={selectedProvider === 'gifs' ? 'Search GIFs...' : 'Search emotes...'}
              className="flex-1 min-w-0 glass-input text-xs px-3 py-1.5 placeholder-textSecondary"
            />
          )}
          {onPopOut && (
            <Tooltip content="Pop out" side="top">
              <button
                onClick={onPopOut}
                aria-label="Pop out the emote menu"
                className="shrink-0 glass-button p-1.5 text-textSecondary hover:text-white transition-colors"
                style={{ borderRadius: '8px' }}
              >
                <ExternalLink size={15} />
              </button>
            </Tooltip>
          )}
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
        {section === 'emotes' && (
        <div ref={setTabRowEl} className="sn-emote-tabs flex gap-1 mt-2">
          {searching && (
            <Tooltip content={`Everything (${tabCount('all')})`} side="top">
              <button onClick={() => setSearchScope('all')} className={tabClass(activeTab === 'all')} style={{ borderRadius: '8px' }}>
                <span className="text-[11px] font-semibold leading-4">All</span>
                <span className="sn-emote-tab-count text-[10px] opacity-70">{fitAmount(tabCount('all'))}</span>
              </button>
            </Tooltip>
          )}
          <Tooltip content={`Favorites (${tabCount('favorites')})`} side="top">
            <button onClick={() => pickTab('favorites')} className={tabClass(activeTab === 'favorites')} style={{ borderRadius: '8px' }}>
              <span className="text-yellow-400">★</span>
              <span className="sn-emote-tab-count text-[10px] opacity-70">{fitAmount(tabCount('favorites'))}</span>
            </button>
          </Tooltip>
          <Tooltip content="Emoji" side="top">
            <button onClick={() => pickTab('emoji')} className={tabClass(activeTab === 'emoji')} style={{ borderRadius: '8px' }}>
              <img src={getAppleEmojiUrl('😀')} alt="😀" className="w-4 h-4" />
              {searching && <span className="sn-emote-tab-count text-[10px] opacity-70">{fitAmount(tabCount('emoji'))}</span>}
            </button>
          </Tooltip>
          {isTwitch && (
            <Tooltip content={`Twitch (${tabCount('twitch')})`} side="top">
              <button onClick={() => pickTab('twitch')} className={tabClass(activeTab === 'twitch')} style={tabStyle(activeTab === 'twitch', 'twitch')}>
                <EmoteProviderLogo provider="twitch" className="w-4 h-4" />
                <span className="sn-emote-tab-count text-[10px] opacity-70">{fitAmount(tabCount('twitch'))}</span>
              </button>
            </Tooltip>
          )}
          {isTwitch && (
            <Tooltip content={`BetterTTV (${tabCount('bttv')})`} side="top">
              <button onClick={() => pickTab('bttv')} className={tabClass(activeTab === 'bttv')} style={tabStyle(activeTab === 'bttv', 'bttv')}>
                <EmoteProviderLogo provider="bttv" className="w-4 h-4" />
                <span className="sn-emote-tab-count text-[10px] opacity-70">{fitAmount(tabCount('bttv'))}</span>
              </button>
            </Tooltip>
          )}
          {isKick && (
            <Tooltip content={`Kick (${tabCount('kick')})`} side="top">
              <button onClick={() => pickTab('kick')} className={tabClass(activeTab === 'kick')} style={tabStyle(activeTab === 'kick', 'kick')}>
                <EmoteProviderLogo provider="kick" className="w-4 h-4" />
                <span className="sn-emote-tab-count text-[10px] opacity-70">{fitAmount(tabCount('kick'))}</span>
              </button>
            </Tooltip>
          )}
          {isYouTube && (
            <Tooltip
              content={
                searching || emotes?.youtube.length
                  ? `YouTube (${tabCount('youtube')})`
                  : 'YouTube (sign in to load this channel’s emoji)'
              }
              side="top"
            >
              <button onClick={() => pickTab('youtube')} className={tabClass(activeTab === 'youtube')} style={tabStyle(activeTab === 'youtube', 'youtube')}>
                <EmoteProviderLogo provider="youtube" className="w-4 h-4" />
                <span className="sn-emote-tab-count text-[10px] opacity-70">{fitAmount(tabCount('youtube'))}</span>
              </button>
            </Tooltip>
          )}
          {/* 7TV supports all three platforms natively, so this tab is not gated to
              Twitch and Kick. A channel whose community does not use 7TV simply
              shows zero, the same as anywhere else. */}
          {(isTwitch || isKick || isYouTube) && (
            <Tooltip content={`7TV (${tabCount('7tv')})`} side="top">
              <button onClick={() => pickTab('7tv')} className={tabClass(activeTab === '7tv')} style={tabStyle(activeTab === '7tv', '7tv')}>
                <EmoteProviderLogo provider="7tv" className="w-4 h-4" />
                <span className="sn-emote-tab-count text-[10px] opacity-70">{fitAmount(tabCount('7tv'))}</span>
              </button>
            </Tooltip>
          )}
          {isTwitch && (
            <Tooltip content={`FrankerFaceZ (${tabCount('ffz')})`} side="top">
              <button onClick={() => pickTab('ffz')} className={tabClass(activeTab === 'ffz')} style={tabStyle(activeTab === 'ffz', 'ffz')}>
                <EmoteProviderLogo provider="ffz" className="w-4 h-4" />
                <span className="sn-emote-tab-count text-[10px] opacity-70">{fitAmount(tabCount('ffz'))}</span>
              </button>
            </Tooltip>
          )}
        </div>
        )}
      </div>
      {/* On the phone the grid fades out against the Emotes/GIFs bar. */}
      <div
        ref={scrollRef}
        className={`flex-1 overflow-y-auto px-2 pb-2 scrollbar-thin ${IS_MOBILE ? 'sn-fade-bottom' : ''}`}
      >
        {searching ? (
          renderSearchResults()
        ) : selectedProvider === 'gifs' ? (
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
          !emojiData ? (
            // The emoji set loads on first open; say so instead of a brief
            // "No emojis found" for a set that has not arrived yet.
            <div className="flex items-center justify-center h-32"><p className="text-xs text-textSecondary">Loading emojis...</p></div>
          ) : emojiSections.length === 0 ? (
            <div className="flex items-center justify-center h-32"><p className="text-xs text-textSecondary">No emojis found</p></div>
          ) : (
            <div className="flex flex-col gap-4 pt-2">
              {emojiSections.map(([category, filteredCategoryEmojis]) => {
                return (
                  <div key={category} className="flex flex-col">
                    <h3 className="text-[10px] text-textSecondary uppercase tracking-wider font-bold mb-2 -mx-2 px-4 sticky top-0 py-1.5 border-b border-white/[0.03] z-10 backdrop-blur-ultra" style={{ backgroundColor: 'color-mix(in srgb, var(--color-background) calc(95% + (1 - var(--glass-strength, 1)) * 5%), transparent)' }}>{category}</h3>
                    {/* Lazy blocks like the emote tabs: all ~1,870 emoji at once was
                        1,870 buttons, tooltips and images on every open. */}
                    {chunkArray(filteredCategoryEmojis, EMOJI_COLS * EMOJI_BLOCK_ROWS).map((block, bi) => (
                      <LazyEmoteBlock
                        key={`${category}-blk-${bi}`}
                        scrollRef={scrollRef}
                        estimatedHeight={Math.ceil(block.length / EMOJI_COLS) * EMOJI_ROW_PX}
                        gridClass="grid grid-cols-8 gap-1 px-1"
                        // Emoji are still images: nothing to animate or freeze.
                        animate={false}
                      >
                        {() =>
                          block.map((emoji, idx) => renderEmojiButton(emoji, `${category}-${bi}-${idx}`))
                        }
                      </LazyEmoteBlock>
                    ))}
                  </div>
                );
              })}
            </div>
          )
        ) : isLoadingEmotes || waitingForRust || (rustTab && answer && !answer.ready) ? (
          <div className="flex items-center justify-center h-32"><p className="text-xs text-textSecondary">Loading emotes...</p></div>
        ) : groups.length === 0 ? (
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
        ) : namedCells ? (
          <div className="flex flex-col gap-4 pt-2">
            {groups.map((group) => ({ groupKey: group.key, group: { name: group.title, emotes: group.emotes } })).map(({ groupKey, group }) => (
              <div key={groupKey} className="flex flex-col">
                <h3 className={`text-[10px] text-textSecondary uppercase tracking-wider font-bold mb-2 -mx-2 px-4 sticky top-0 py-1.5 border-b border-borderSubtle z-10 backdrop-blur-ultra${IS_MOBILE ? ' flex items-center gap-1.5' : ''}`} style={{ backgroundColor: 'color-mix(in srgb, var(--color-background) calc(95% + (1 - var(--glass-strength, 1)) * 5%), transparent)' }}>
                  {IS_MOBILE && <SectionMark tab={selectedProvider} sectionKey={groupKey} />}
                  <span className="text-textPrimary">{group.name}</span> <span className="opacity-50">{IS_MOBILE ? group.emotes.length : `(${group.emotes.length})`}</span>
                </h3>
                {chunkArray(group.emotes, TWITCH_COLS * TWITCH_BLOCK_ROWS).map((block, bi) => {
                  const rows = Math.ceil(block.length / TWITCH_COLS);
                  return (
                    <LazyEmoteBlock
                      key={`${groupKey}-blk-${bi}`}
                      scrollRef={scrollRef}
                      estimatedHeight={rows * TWITCH_ROW_PX}
                      gridClass="grid grid-cols-7 gap-2 px-1"
                      animate={animateGrid}
                      onActivate={() => {
                        const tier = inlineEmoteTier();
                        for (const e of block) queueEmoteForDisplayCaching(e.id, e.provider, e.url, tier, true);
                      }}
                    >
                      {(still) =>
                        block.map((emote, idx) => {
                          const isFavorited = isFavoriteEmote(emote.id);
                          const stillKey = `${emote.provider}:${emote.id}`;
                          const drawStill = still && emote.animated !== false && !noStillFrame.has(stillKey);
                          const liveSrc = drawStill
                            ? staticEmoteUrl(emote.url)
                            : getCachedEmoteUrl(emote.id, emote.provider) || emote.localUrl || emote.url;
                          return (
                            <div key={`${groupKey}-${emote.provider}-${emote.id}-${idx}`} className="relative group">
                              <Tooltip
                                side="top"
                                delay={200}
                                content={
                                  <div className="flex flex-col items-center gap-1.5 py-0.5">
                                    <img
                                      src={hoverImageUrl(emote)}
                                      alt={emote.name}
                                      className="w-auto object-contain mx-auto drop-shadow-md"
                                      style={{ height: hoverPreviewSize, maxWidth: hoverPreviewSize * 2 }}
                                      onError={(e) => {
                                        const t = e.currentTarget;
                                        if (t.src !== emote.url) t.src = emote.url;
                                      }}
                                    />
                                    <div className="text-center flex flex-col items-center gap-0.5">
                                      <span className="font-bold text-[13px] leading-tight">{emote.name}</span>
                                      <EmoteCreditLine
                                        provider={emote.provider}
                                        id={emote.id}
                                        name={emote.name}
                                        ownerHint={emote.owner_name}
                                        originalHint={emote.original_name}
                                        channelLogin={channelLogin}
                                        channelId={channelId}
                                      />
                                    </div>
                                  </div>
                                }
                              >
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
                                      if (drawStill) noStillFrame.add(stillKey);
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
            {groups
              .map((g) => ({ label: g.title, key: g.key, emotes: g.emotes, cols: g.columns, gridCols: GRID_COLS[g.columns] ?? 'grid-cols-7' }))
              .map((group) => (
                <div key={group.key} className="flex flex-col">
                  <h3 className={`text-[10px] text-textSecondary uppercase tracking-wider font-bold mb-2 -mx-2 px-4 sticky top-0 py-1.5 border-b border-borderSubtle z-10 backdrop-blur-ultra${IS_MOBILE ? ' flex items-center gap-1.5' : ''}`} style={{ backgroundColor: 'color-mix(in srgb, var(--color-background) calc(95% + (1 - var(--glass-strength, 1)) * 5%), transparent)' }}>
                    {IS_MOBILE && <SectionMark tab={selectedProvider} sectionKey={group.key} />}
                    <span className="text-textPrimary">{group.label}</span> <span className="opacity-50">{IS_MOBILE ? group.emotes.length : `(${group.emotes.length})`}</span>
                  </h3>
                  {chunkArray(group.emotes, group.cols * WIDTH_BLOCK_ROWS).map((block, bi) => {
                    const rows = Math.ceil(block.length / group.cols);
                    return (
                      <LazyEmoteBlock
                        key={`${group.key}-blk-${bi}`}
                        scrollRef={scrollRef}
                        estimatedHeight={rows * WIDTH_ROW_PX}
                        gridClass={`grid ${group.gridCols} gap-2 px-1`}
                        animate={animateGrid}
                        onActivate={() => {
                          const tier = inlineEmoteTier();
                          for (const e of block) queueEmoteForDisplayCaching(e.id, e.provider, e.url, tier, true);
                        }}
                      >
                        {(still) =>
                          block.map((emote: PickerCell, idx: number) => {
                            const isFavorited = isFavoriteEmote(emote.id);
                            return (
                              <EmoteGridItem
                                key={`${emote.provider}-${emote.id}-${idx}`}
                                emote={emote}
                                isFavorited={isFavorited}
                                still={still}
                                replacedBy={emote.replaced_by}
                                onInsert={() => onInsert(emote.insertText ?? emote.name)}
                                onToggleFavorite={() => void toggleFavorite(emote, isFavorited)}
                                channelLogin={channelLogin}
                                channelId={channelId}
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
      </div>
      {sections.length > 1 && <MenuSwitcher label="Insert" tabs={sections} value={section} onChange={pickSection} />}
    </motion.div>
  );
}
