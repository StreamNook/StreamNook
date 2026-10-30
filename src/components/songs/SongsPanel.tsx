// Your songs: every song you identified, newest heard first. One body for both
// shells, the desktop's modal (SongsPanelHost) and the phone's You > Songs
// screen (mobile/screens/SongsScreen). Rust keeps, searches and pages the songs
// (services/song_id/history.rs) and keeps each cover's colors
// (cover_tint.rs); this draws a window of the list (the newest song as the
// Latest card, then the songs in time groups) and one song's detail over the
// list, which stays mounted so it keeps its scroll. A song's page takes its
// cover's hue as a wash over the whole panel (utils/coverTint.ts fits it to
// the theme, so every word stays as readable as the theme makes it).
import {
  memo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
} from 'react';
import './songsScroll.css';
import { VariableSizeList as List, type ListChildComponentProps } from 'react-window';
import { AnimatePresence, motion } from 'framer-motion';
import {
  ArrowLeft,
  ArrowUp,
  CalendarClock,
  Check,
  ChevronRight,
  Copy,
  Disc3,
  Headphones,
  History,
  Link2,
  Music,
  RadioTower,
  Repeat,
  Search,
  Trash2,
  X,
  type LucideIcon,
} from 'lucide-react';
import { ArrowLeft as PhoneArrowLeft, Trash as PhoneTrash } from 'phosphor-react';
import { useAppStore } from '../../stores/AppStore';
import { subscribeTheme, themeVersion } from '../../themes';
import { AUDIO_GRAPH_SUPPORTED } from '../../utils/audioBoost';
import {
  CARD_LAYERS,
  readSurface,
  rowTint,
  useCoverFit,
  washBackground,
  type CoverTint,
  type Fit,
  type SurfaceKind,
} from '../../utils/coverTint';
import { Logger } from '../../utils/logger';
import { IS_MOBILE } from '../../utils/platform';
import {
  clearSongHistory,
  detailChange,
  fetchSong,
  onSongHistoryChanged,
  removeSong,
  restoreSong,
  type SongEntry,
  type SongRow,
  type SongSource,
} from '../../utils/songHistory';
import { openSongLink, refreshSongApps } from '../../utils/songLinks';
import { albumLine, channelOf, heardAgo, heardClock, heardDay, heardInto, mediaMark } from '../../utils/songWords';
import { ProviderLogo } from '../ProviderLogo';
import { Tooltip } from '../ui/Tooltip';
import { ACCENT_BUTTON, DANGER_FILL, GLAZE_PILL } from '../ui/glazeButtons';
import { SongLinkButtons } from './SongLinkButtons';
import { anchorDelta, makeLayout, ROW_HEIGHT, type Layout } from './songGroups';
import { songLinks } from './songLinks';
import { useSongPages } from './useSongPages';

/** Row height, art size and type for each shell: the phone's rows are roomier,
 *  like the You screen's. */
const ROW = {
  modal: { height: ROW_HEIGHT.modal, art: 52, title: 'text-[14px]', artist: 'text-[12.5px]', heard: 'text-[12px]' },
  screen: { height: ROW_HEIGHT.screen, art: 56, title: 'text-[15px]', artist: 'text-[13px]', heard: 'text-[12.5px]' },
} as const;

const plural = (n: number, one: string, many: string) => (n === 1 ? `1 ${one}` : `${n.toLocaleString('en-US')} ${many}`);
const songsCount = (n: number) => plural(n, 'song', 'songs');

/** The accent's own tint, for the marks that head the panel and its sections. */
const MARK_STYLE = {
  background:
    'linear-gradient(150deg, color-mix(in srgb, var(--color-accent) 34%, transparent), color-mix(in srgb, var(--color-accent) 12%, transparent))',
  boxShadow:
    'inset 0 1px 0 rgba(255, 255, 255, 0.14), inset 0 0 0 1px color-mix(in srgb, var(--color-accent) 28%, transparent)',
  color: 'var(--color-accent-on-surface)',
} as const;

/** The Your songs heading, in Inter (loaded app-wide). */
const HEADING: CSSProperties = {
  fontFamily: '"Inter", var(--app-font, sans-serif)',
  fontWeight: 780,
  letterSpacing: '-0.02em',
};

/** The house easing for the panel's moves. */
const EASE = [0.2, 0.8, 0.2, 1] as const;

/** The flyout's own top light (globals.css .glass-flyout), for a still layer
 *  that has to match the panel at its place. */
const FLYOUT_TOP_LIGHT =
  'linear-gradient(180deg, rgba(255, 255, 255, 0.05), rgba(255, 255, 255, 0.012) 45%, transparent)';

/** A size-change-safe fade height for the scroller edges. */
const FADE_PX = 16;

const serverVersion = () => 0;
/** The theme's version: colors fitted to it are fitted again when it moves. */
const useThemeVersion = () => useSyncExternalStore(subscribeTheme, themeVersion, serverVersion);

// ---------------------------------------------------------------- measuring

interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}
const NO_BOX: Box = { x: 0, y: 0, w: 0, h: 0 };

/** Where `el` sits inside `root`, from layout offsets, so the dialog's open
 *  animation (a scale) never skews it. */
function offsetWithin(el: HTMLElement, root: HTMLElement): { x: number; y: number } {
  let x = 0;
  let y = 0;
  let at: HTMLElement | null = el;
  while (at && at !== root) {
    x += at.offsetLeft;
    y += at.offsetTop;
    at = at.offsetParent as HTMLElement | null;
  }
  return at === root ? { x, y } : { x: 0, y: 0 };
}

/** An element's content box, and where it sits inside `root` when given.
 *  Takes the element itself (held in state by its ref), so one that mounts
 *  later is measured once it does. */
function useBox(el: HTMLElement | null, root: HTMLElement | null = null): Box {
  const [box, setBox] = useState<Box>(NO_BOX);
  useEffect(() => {
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      const rect = entries[entries.length - 1].contentRect;
      const at = root ? offsetWithin(el, root) : { x: 0, y: 0 };
      const next = { x: at.x, y: at.y, w: Math.round(rect.width), h: Math.round(rect.height) };
      setBox((was) => (was.x === next.x && was.y === next.y && was.w === next.w && was.h === next.h ? was : next));
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [el, root]);
  return box;
}

/**
 * Whether a cover's colors came after the song was first drawn (Rust was still
 * reading the cover): the wash then fades in once instead of popping. Colors
 * already there when the song first draws show at once.
 */
function useLateTint(tint: CoverTint | null | undefined, drawn: boolean): boolean {
  const has = !!tint?.main;
  const [atFirst, setAtFirst] = useState<boolean | null>(null);
  if (drawn && atFirst === null) setAtFirst(has);
  return has && atFirst === false;
}

// ---------------------------------------------------------------- pieces

/** A cover, or the music mark when there is none or it will not load. Tries
 *  `fallback` (the full cover) when a small copy is refused, and fades in once
 *  it has loaded so it never paints in as a strip. Callers key it by `src`, so
 *  a new cover starts over. */
function SongArt({
  src,
  fallback,
  px,
  round = 'rounded-lg',
}: {
  src: string | null;
  fallback?: string | null;
  px: number;
  round?: string;
}) {
  const [step, setStep] = useState(0);
  const [loaded, setLoaded] = useState(false);
  const url = step === 0 ? src : step === 1 ? (fallback ?? null) : null;
  if (!url) {
    return (
      <div
        className={`flex shrink-0 items-center justify-center bg-glass-hover ${round}`}
        style={{ width: px, height: px }}
        aria-hidden="true"
      >
        <Music size={Math.round(px * 0.38)} className="text-textMuted" />
      </div>
    );
  }
  return (
    <span className={`relative block shrink-0 bg-glass-hover ${round}`} style={{ width: px, height: px }}>
      <img
        src={url}
        alt=""
        width={px}
        height={px}
        loading="lazy"
        decoding="async"
        onLoad={() => setLoaded(true)}
        onError={() => {
          setLoaded(false);
          setStep((s) => (s === 0 && fallback ? 1 : 2));
        }}
        className={`absolute inset-0 h-full w-full object-cover transition-opacity duration-150 ${round} ${loaded ? 'opacity-100' : 'opacity-0'}`}
      />
    </span>
  );
}

/** A channel's picture, or its first letter when it has none or it will not
 *  load. Callers key it by `src`. */
function ChannelFace({ src, name, px }: { src?: string | null; name: string; px: number }) {
  const [failed, setFailed] = useState(false);
  const [loaded, setLoaded] = useState(false);
  if (!src || failed) {
    return (
      <span
        className="flex shrink-0 items-center justify-center rounded-full bg-glass-hover font-semibold leading-none text-textSecondary"
        style={{ width: px, height: px, fontSize: Math.max(8, Math.round(px * 0.46)) }}
        aria-hidden="true"
      >
        {name.charAt(0).toUpperCase()}
      </span>
    );
  }
  return (
    <span className="relative block shrink-0 rounded-full bg-glass-hover" style={{ width: px, height: px }} aria-hidden="true">
      <img
        src={src}
        alt=""
        width={px}
        height={px}
        loading="lazy"
        decoding="async"
        onLoad={() => setLoaded(true)}
        onError={() => setFailed(true)}
        className={`absolute inset-0 h-full w-full rounded-full object-cover transition-opacity duration-150 ${loaded ? 'opacity-100' : 'opacity-0'}`}
      />
    </span>
  );
}

/** "VOD" or "Clip" beside a channel heard on one. */
function MediaChip({ source }: { source: SongSource }) {
  const mark = mediaMark(source);
  if (!mark) return null;
  return (
    <span className="shrink-0 rounded-[5px] bg-glass-hover px-1.5 py-px text-[11px] font-semibold leading-[15px] text-textSecondary">
      {mark}
    </span>
  );
}

/** A cover's wash as a still layer over its box. `fadeIn` fades it in once
 *  (its colors came after the song was drawn): opacity only, on the
 *  compositor. */
function Wash({ background, fadeIn }: { background: string; fadeIn: boolean }) {
  if (!fadeIn) return <div aria-hidden="true" className="pointer-events-none absolute inset-0" style={{ background }} />;
  return (
    <motion.div
      aria-hidden="true"
      className="pointer-events-none absolute inset-0"
      style={{ background }}
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: 0.3, ease: 'easeOut' }}
    />
  );
}

/** What the surface paints under a box at `y`: the phone's page, or the
 *  flyout's solid base with its top light placed where the box sits. */
function baseUnder(screen: boolean, surfaceH: number, y: number): string {
  if (screen) return 'var(--color-background)';
  if (!(surfaceH > 0)) return 'var(--flyout-heading-fill)';
  return `${FLYOUT_TOP_LIGHT} 0px ${-y}px / 100% ${surfaceH}px no-repeat, var(--flyout-heading-fill)`;
}

/** Fades what scrolls past an edge of a scroller into the surface under it,
 *  instead of cutting it flat. A still layer over the scroller, so scrolling
 *  repaints nothing: the surface as it is painted there (and the cover's wash
 *  over it on a song's page, cut to the same box so the two line up), masked
 *  into a gradient. */
function EdgeFade({
  edge,
  base,
  wash = null,
  fadeIn = false,
  shown = true,
  offset = '0px',
}: {
  edge: 'top' | 'bottom';
  base: string;
  wash?: string | null;
  fadeIn?: boolean;
  shown?: boolean;
  /** How far in from the edge the scroller ends (the phone's safe area). */
  offset?: string;
}) {
  const mask = `linear-gradient(${edge === 'top' ? 'to top' : 'to bottom'}, transparent, #000)`;
  return (
    <div
      aria-hidden="true"
      className={`pointer-events-none absolute inset-x-0 h-4 transition-opacity duration-150 ${shown ? 'opacity-100' : 'opacity-0'}`}
      style={{ [edge]: offset, maskImage: mask, WebkitMaskImage: mask }}
    >
      <div className="absolute inset-0" style={{ background: base }} />
      {wash && <Wash background={wash} fadeIn={fadeIn} />}
    </div>
  );
}

// ---------------------------------------------------------------- the list

interface RowData {
  layout: Layout;
  rowAt: (row: number) => SongRow | null;
  open: (key: string) => void;
  /** The list mixes platforms: each place carries its platform's logo. */
  mixed: boolean;
  screen: boolean;
  kind: SurfaceKind;
  /** The theme's version, so drawn tints follow a theme change. */
  version: number;
}

/** The matched song an item shows or heads (a header heads its first row). */
function rowOfItem(layout: Layout, index: number): number {
  const item = layout.item(index);
  if (item.kind === 'row') return item.row;
  if (item.kind === 'latest') return 0;
  const next = index + 1 < layout.count ? layout.item(index + 1) : null;
  return next?.kind === 'row' ? next.row : 0;
}

/** Where a song was last heard, under its name. */
function HeardLine({ heard, mixed, className }: { heard: SongSource; mixed: boolean; className: string }) {
  return (
    <span className={`flex min-w-0 items-center gap-1.5 text-textMuted ${className}`}>
      <ChannelFace key={heard.avatar ?? ''} src={heard.avatar} name={channelOf(heard)} px={15} />
      {/* One run of text, so a cut takes the category before the channel. */}
      <span className="min-w-0 truncate">
        <span className="text-textSecondary">{channelOf(heard)}</span>
        {heard.category && <span> · {heard.category}</span>}
      </span>
      {mixed && <ProviderLogo provider={heard.provider} size={11} className="shrink-0" />}
      <MediaChip source={heard} />
    </span>
  );
}

/** The newest song, on its own cover-colored card above the groups. */
function LatestCard({ row, data }: { row: SongRow; data: RowData }) {
  const [host, setHost] = useState<HTMLDivElement | null>(null);
  const box = useBox(host);
  const fit = useCoverFit(row.tint, data.kind, box.w, box.h, CARD_LAYERS, true);
  const late = useLateTint(row.tint, true);
  const ink = fit?.ink ?? 'var(--color-accent-on-surface)';
  const big = row.art_full ?? row.art;
  return (
    // Under the pointer it lifts a pixel over a slightly deeper shadow (its
    // own still layer, faded in: no shadow is animated).
    <div className="relative isolate h-full">
    <button
      type="button"
      data-song-row={0}
      data-song-key={row.key}
      onClick={() => data.open(row.key)}
      className="glass-tile peer relative flex h-full w-full items-center gap-4 overflow-hidden !rounded-2xl p-2.5 pr-3.5 text-left outline-none transition-transform duration-200 ease-[cubic-bezier(0.2,0.8,0.2,1)] focus-visible:outline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-[color:var(--card-ink)] [html[data-motion=full]_&:active]:scale-[0.985] [html[data-motion=full]_&:hover]:-translate-y-px"
      style={{ '--card-ink': ink } as CSSProperties}
    >
      <div ref={setHost} aria-hidden="true" className="pointer-events-none absolute inset-0">
        {fit && <Wash background={washBackground(fit, box, { x: 0, y: 0, w: box.w, h: box.h })} fadeIn={late} />}
      </div>
      {/* Above the wash: the cover is never tinted. */}
      <span
        className="relative shrink-0 rounded-xl transition-shadow duration-300"
        style={{ boxShadow: `0 12px 28px -14px ${fit?.glow ?? 'rgba(0, 0, 0, 0.8)'}` }}
      >
        <SongArt key={big ?? ''} src={big} fallback={row.art} px={80} round="rounded-xl" />
      </span>
      <span className="relative flex min-w-0 flex-1 flex-col">
        <span className="text-[11px] font-semibold uppercase leading-[15px] tracking-[0.06em]" style={{ color: ink }}>
          Latest · {heardAgo(row.last_ms)}
        </span>
        <span className="mt-1 truncate text-[16px] font-bold leading-[21px] tracking-tight text-textPrimary">{row.title}</span>
        <span className="truncate text-[13px] font-medium leading-[18px] text-textSecondary">{row.artist}</span>
        {row.heard && <HeardLine heard={row.heard} mixed={data.mixed} className="mt-1 text-[12px] leading-[17px]" />}
      </span>
      <ChevronRight size={18} className="relative shrink-0 text-textMuted" aria-hidden="true" />
    </button>
    <div
      aria-hidden="true"
      className="pointer-events-none absolute inset-0 -z-10 rounded-2xl opacity-0 shadow-[0_16px_30px_-16px_rgba(0,0,0,0.7)] transition-opacity duration-200 [@media(hover:hover)]:peer-hover:opacity-100"
    />
    </div>
  );
}

/** A time group's name over its songs, and how many it holds. */
function GroupHeader({ style, label, count, screen }: { style: CSSProperties; label: string; count: number; screen: boolean }) {
  return (
    // Its edges line up with the rows' covers and times below it.
    <div style={style} className={`flex items-end pb-2 ${screen ? 'pl-4 pr-2.5' : 'pl-5 pr-3.5'}`}>
      <h3 className="flex min-w-0 flex-1 items-center gap-2 text-[11.5px] font-semibold uppercase leading-4 tracking-[0.06em] text-textMuted">
        <span className="min-w-0 truncate">{label}</span>
        <span className="ml-auto shrink-0 font-medium normal-case tracking-normal tabular-nums">{songsCount(count)}</span>
      </h3>
    </div>
  );
}

function SongListRow({ style, row, index, data }: { style: CSSProperties; row: SongRow | null; index: number; data: RowData }) {
  const size = data.screen ? ROW.screen : ROW.modal;
  // The rows' covers and times line up with the search field above them.
  const inset = data.screen ? 'pl-1.5' : 'pl-2.5 pr-1';
  if (!row) {
    return (
      <div style={style} className={inset} aria-hidden="true">
        <div className="flex h-full items-center gap-3.5 px-2.5">
          <div className="shrink-0 rounded-lg bg-glass-hover" style={{ width: size.art, height: size.art }} />
          <div className="flex min-w-0 flex-1 flex-col gap-2">
            <div className="h-3 w-2/5 rounded bg-glass-hover" />
            <div className="h-2.5 w-1/3 rounded bg-glass-hover opacity-70" />
            <div className="h-2.5 w-1/2 rounded bg-glass-hover opacity-70" />
          </div>
        </div>
      </div>
    );
  }
  // Its own cover's color on hover and keyboard focus, where there is a
  // pointer (a phone's rows open on a tap); the ordinary hover when no tint of
  // that cover keeps the muted words readable.
  const tint = data.screen ? null : rowTint(row.tint, readSurface(data.kind));
  return (
    <div style={style} className={inset}>
      <button
        type="button"
        data-song-row={index}
        data-song-key={row.key}
        onClick={() => data.open(row.key)}
        style={{ '--row-tint': tint ?? 'var(--color-surface-hover)' } as CSSProperties}
        className="flex h-full w-full items-center gap-3.5 rounded-xl px-2.5 text-left outline-none transition-[background-color,transform] duration-150 focus-visible:bg-[color:var(--row-tint)] [@media(hover:hover)]:hover:bg-[color:var(--row-tint)] [html[data-motion=full]_&]:active:scale-[0.985]"
      >
        <SongArt key={row.art ?? ''} src={row.art} fallback={row.art_full} px={size.art} />
        <span className="flex min-w-0 flex-1 flex-col gap-px">
          <span className="flex items-baseline gap-3">
            <span className={`min-w-0 flex-1 truncate font-semibold leading-[19px] text-textPrimary ${size.title}`}>
              {row.title}
            </span>
            <span className="shrink-0 text-[11.5px] tabular-nums text-textMuted">{heardAgo(row.last_ms)}</span>
          </span>
          <span className={`truncate leading-[17px] text-textSecondary ${size.artist}`}>{row.artist}</span>
          {row.heard && (
            <span className="flex min-w-0 items-center">
              <HeardLine heard={row.heard} mixed={data.mixed} className={`flex-1 leading-[17px] ${size.heard}`} />
              {row.count > 1 && (
                <span className="flex shrink-0 items-center gap-1 pl-2 text-[11px] font-medium tabular-nums text-textMuted">
                  <Repeat size={11} strokeWidth={2.25} aria-hidden="true" />
                  {row.count.toLocaleString('en-US')}
                  <span className="sr-only"> times</span>
                </span>
              )}
            </span>
          )}
        </span>
      </button>
    </div>
  );
}

/** One item of the list: the Latest card, a group's header, or a song. */
const ListItem = memo(function ListItem({ index, style, data }: ListChildComponentProps<RowData>) {
  const item = data.layout.item(index);
  if (item.kind === 'latest') {
    const row = data.rowAt(0);
    return (
      <div style={style} className={`pb-2 pt-1 ${data.screen ? 'pl-1.5' : 'pl-2.5 pr-1'}`}>
        {row ? (
          <LatestCard key={row.key} row={row} data={data} />
        ) : (
          <div className="glass-tile h-full w-full !rounded-2xl" aria-hidden="true" />
        )}
      </div>
    );
  }
  if (item.kind === 'header') return <GroupHeader style={style} label={item.label} count={item.count} screen={data.screen} />;
  return <SongListRow style={style} row={data.rowAt(item.row)} index={item.row} data={data} />;
});

function EmptyState({ searched }: { searched: string | null }) {
  if (searched) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 px-8 pb-10 text-center">
        <Search size={30} strokeWidth={1.9} className="text-textMuted" aria-hidden="true" />
        <div className="flex flex-col gap-1">
          <p className="text-[14px] font-semibold text-textPrimary">No songs match “{searched}”</p>
          <p className="text-[12.5px] text-textMuted">Search by song, artist, channel or category.</p>
        </div>
      </div>
    );
  }
  return (
    <div className="flex h-full flex-col items-center justify-center gap-4 px-8 pb-10 text-center">
      {/* The player's own music note, so it teaches the button. */}
      <Music size={38} strokeWidth={1.8} className="text-textMuted" aria-hidden="true" />
      <div className="flex max-w-[280px] flex-col gap-1.5">
        <p className="text-[15px] font-semibold text-textPrimary">Songs you identify land here</p>
        {AUDIO_GRAPH_SUPPORTED && (
          <p className="text-[12.5px] leading-[19px] text-textMuted [text-wrap:balance]">
            {IS_MOBILE ? 'Type ' : 'Press the music note on the player, or type '}
            <kbd className="rounded-md border border-borderSubtle bg-glass-hover px-1.5 py-px font-mono text-[11.5px] text-textSecondary">
              /song
            </kbd>
            {IS_MOBILE ? ' in a chat while a stream plays.' : ' in chat.'}
          </p>
        )}
      </div>
    </div>
  );
}

/** A load that failed, with the one way forward. */
function LoadFailed({ what, onRetry }: { what: string; onRetry: () => void }) {
  return (
    <div className="flex h-full flex-1 flex-col items-center justify-center gap-3 px-8 text-center">
      <p className="text-[13.5px] text-textSecondary">{what}</p>
      <button type="button" onClick={onRetry} className={`${GLAZE_PILL} h-8 px-3.5 text-[12.5px]`}>
        Try again
      </button>
    </div>
  );
}

/** The panel's Close, in the same place on the list and on a song. */
function CloseButton({ onClose }: { onClose: () => void }) {
  return (
    <Tooltip content="Close" side="bottom">
      <button
        type="button"
        onClick={onClose}
        aria-label="Close"
        className="-mr-2.5 flex h-8 w-8 items-center justify-center rounded-full text-textMuted transition-colors hover:text-textPrimary"
      >
        <X size={20} />
      </button>
    </Tooltip>
  );
}

// ---------------------------------------------------------------- a song's page

/** A song's top bar. The phone keeps the list's own header row and arrow (the
 *  song's title below names the page); the modal keeps its Close where the
 *  list has it. Both put the arrow and Close exactly where the list's header
 *  does, so nothing moves going in and out of a song. */
function BackBar({ screen, onBack, onClose }: { screen: boolean; onBack: () => void; onClose: () => void }) {
  if (screen) {
    return (
      <div className="flex h-14 shrink-0 items-center pl-1 pr-[2.5px]">
        <button
          type="button"
          onClick={onBack}
          aria-label="Back to Songs"
          className="sn-touch -mr-1 flex items-center justify-center text-textSecondary"
        >
          <PhoneArrowLeft size={22} />
        </button>
      </div>
    );
  }
  return (
    <div className="flex shrink-0 items-center gap-2 pl-[9px] pr-5 pt-[13px]">
      <button
        type="button"
        onClick={onBack}
        className="flex h-8 items-center gap-1.5 rounded-lg px-2 text-[13px] font-medium text-textSecondary transition-colors hover:bg-glass-hover hover:text-textPrimary"
      >
        <ArrowLeft size={18} />
        Your songs
      </button>
      <span className="flex-1" />
      <CloseButton onClose={onClose} />
    </div>
  );
}

/** A part of a song's page: its mark (in the cover's ink when it has one) and
 *  name over what it holds, and a quiet action at the end of the name's line
 *  when it has one. */
function Section({
  icon: Icon,
  title,
  mark,
  aside,
  children,
}: {
  icon: LucideIcon;
  title: string;
  mark: Fit['mark'] | null;
  aside?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="mt-6">
      <div className="mb-2.5 flex items-center gap-2">
        <span
          className="flex h-[22px] w-[22px] shrink-0 items-center justify-center rounded-[7px]"
          style={mark ?? MARK_STYLE}
          aria-hidden="true"
        >
          <Icon size={12.5} strokeWidth={2.3} />
        </span>
        <h3 className="text-[13px] font-semibold text-textPrimary">{title}</h3>
        {aside}
      </div>
      {children}
    </section>
  );
}

/** One fact in the song's facts card: its name over its value, which starts
 *  with a capital beside the others ("Just now" by "Yesterday"). */
function Fact({ icon: Icon, label, value, ink }: { icon: LucideIcon; label: string; value: string; ink: string | null }) {
  return (
    <div className="flex min-w-0 flex-col gap-1 px-3 py-2.5">
      <span className="flex items-center gap-1.5 text-[11px] font-medium text-textMuted">
        <Icon size={12} strokeWidth={2.2} aria-hidden="true" className="shrink-0" style={ink ? { color: ink } : undefined} />
        <span className="truncate">{label}</span>
      </span>
      <span className="whitespace-nowrap text-[14px] font-semibold tabular-nums text-textPrimary">
        {value.charAt(0).toUpperCase() + value.slice(1)}
      </span>
    </div>
  );
}

/** One time the song was heard: the channel, the stream it was on, and when. */
function Place({ at, source, mixed }: { at: number; source: SongSource; mixed: boolean }) {
  const name = channelOf(source);
  const into = heardInto(source);
  const meta = [source.category, into].filter(Boolean).join(' · ');
  return (
    <div className="flex items-center gap-3 px-3.5 py-3">
      <ChannelFace key={source.avatar ?? ''} src={source.avatar} name={name} px={36} />
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <div className="flex min-w-0 items-center gap-1.5">
          <span className="truncate text-[13.5px] font-semibold leading-[18px] text-textPrimary">{name}</span>
          {mixed && <ProviderLogo provider={source.provider} size={12} className="shrink-0" />}
          <MediaChip source={source} />
        </div>
        {source.title && (
          <Tooltip content={source.title} side="top" describe>
            <span className="truncate text-[12.5px] leading-[17px] text-textSecondary">{source.title}</span>
          </Tooltip>
        )}
        {meta && <span className="truncate text-[12px] leading-[16px] text-textMuted">{meta}</span>}
      </div>
      <div className="flex shrink-0 flex-col items-end gap-0.5 self-start pt-px">
        <span className="text-[12.5px] font-medium leading-[18px] tabular-nums text-textSecondary">{heardDay(at)}</span>
        <span className="text-[11.5px] leading-[16px] tabular-nums text-textMuted">{heardClock(at)}</span>
      </div>
    </div>
  );
}

function SongDetail({
  songKey,
  mixed,
  screen,
  root,
  surface,
  onBack,
  onClose,
}: {
  songKey: string;
  mixed: boolean;
  screen: boolean;
  /** The panel, whose whole box (`surface`) the wash covers. */
  root: HTMLElement | null;
  surface: Box;
  onBack: () => void;
  onClose: () => void;
}) {
  // undefined while loading, null once it is gone (removed, cleared, never kept).
  const [song, setSong] = useState<SongEntry | null | undefined>(undefined);
  const [failed, setFailed] = useState(false);
  // Bumped by Try again, which asks once more.
  const [attempt, setAttempt] = useState(0);
  const [copied, setCopied] = useState(false);
  const [scrolledDown, setScrolledDown] = useState(false);

  useEffect(() => {
    let alive = true;
    // Every ask takes a ticket and only the latest answer counts, so a slow
    // answer never brings back a song removed while it was on its way.
    let asked = 0;
    const load = () => {
      const ticket = ++asked;
      fetchSong(songKey)
        .then((found) => {
          if (!alive || ticket !== asked) return;
          setFailed(false);
          setSong(found);
        })
        .catch((e: unknown) => {
          Logger.warn('[Songs] song failed:', e);
          if (alive && ticket === asked) setFailed(true);
        });
    };
    load();
    // Read again when it is filed again or its cover's colors land; gone
    // when it is removed or the list is cleared.
    const unlisten = onSongHistoryChanged((change) => {
      const what = detailChange(change, songKey);
      if (what === 'gone') {
        asked += 1;
        if (alive) setSong(null);
      } else if (what === 'reload') {
        load();
      }
    });
    return () => {
      alive = false;
      void unlisten.then((stop) => stop());
    };
  }, [songKey, attempt]);

  useEffect(() => {
    if (song === null) onBack();
  }, [song, onBack]);

  useEffect(() => {
    if (!copied) return;
    const t = window.setTimeout(() => setCopied(false), 1500);
    return () => window.clearTimeout(t);
  }, [copied]);

  const kind: SurfaceKind = screen ? 'screen' : 'flyout';
  const fit = useCoverFit(song?.tint, kind, surface.w, surface.h);
  const late = useLateTint(song?.tint, song !== undefined);
  const [bodyEl, setBodyEl] = useState<HTMLDivElement | null>(null);
  const body = useBox(bodyEl, root);

  if (failed && song === undefined) {
    return (
      <div className="absolute inset-0 flex min-h-0 flex-col">
        <BackBar screen={screen} onBack={onBack} onClose={onClose} />
        <LoadFailed what="Couldn't open this song." onRetry={() => setAttempt((n) => n + 1)} />
      </div>
    );
  }
  if (!song) return null;

  const links = songLinks(song);
  const album = albumLine(song.album, song.title);
  const places = song.sightings.flatMap((s) => (s.source ? [{ at: s.at_ms, source: s.source }] : []));
  const cover = screen ? 116 : 152;
  const ink = fit?.ink ?? null;
  const mark = fit?.mark ?? null;
  // The wash cut to a strip of the panel at `y`, for the edge fades.
  const washAt = (y: number) => (fit ? washBackground(fit, surface, { x: 0, y, w: surface.w, h: FADE_PX }) : null);
  const bodyTop = body.y;
  const bodyBottom = body.y + body.h;

  const copyName = () => {
    navigator.clipboard
      .writeText(`${song.title} - ${song.artist}`)
      .then(() => setCopied(true))
      .catch(() => useAppStore.getState().addToast('Could not copy the song name', 'error'));
  };
  const remove = () => {
    const { key, title } = song;
    removeSong(key)
      .then((removed) => {
        if (!removed) return;
        useAppStore.getState().addToast(
          `Removed “${title}”`,
          'success',
          {
            label: 'Undo',
            onClick: () => {
              restoreSong(key).catch((e: unknown) => {
                Logger.warn('[Songs] undo failed:', e);
                useAppStore.getState().addToast('Could not bring the song back', 'error');
              });
            },
          },
          // An Undo always shows, and is not kept in the island.
          { alwaysShow: true, skipIsland: true },
        );
      })
      .catch((e: unknown) => {
        Logger.warn('[Songs] remove failed:', e);
        useAppStore.getState().addToast('Could not remove the song', 'error');
      });
  };

  return (
    <div className="absolute inset-0 flex min-h-0 flex-col">
      {/* The cover's hue over the whole panel, under everything on it. */}
      {fit && <Wash background={washBackground(fit, surface, { x: 0, y: 0, w: surface.w, h: surface.h })} fadeIn={late} />}
      <div className="relative flex min-h-0 flex-1 flex-col">
        <BackBar screen={screen} onBack={onBack} onClose={onClose} />
        <div ref={setBodyEl} className="relative flex min-h-0 flex-1 flex-col">
          <div
            className={`sn-songs-scroll min-h-0 flex-1 overflow-y-auto ${screen ? 'px-4' : 'pb-6 pl-5 pr-[14px]'}`}
            style={screen ? { paddingBottom: 'calc(var(--sn-safe-b, 0px) + 24px)' } : undefined}
            onScroll={(e) => setScrolledDown(e.currentTarget.scrollTop > 0)}
          >
            <div className="flex items-start gap-5 pt-4">
              {/* The cover sits on the wash with its own plain shadow, and
                  starts low enough that the scroller's top edge never cuts it. */}
              <span
                className="relative block shrink-0 rounded-xl shadow-[0_12px_28px_-14px_rgba(0,0,0,0.8)] ring-1 ring-inset ring-white/10"
                style={{ width: cover, height: cover }}
              >
                <SongArt key={song.album_art ?? ''} src={song.album_art} px={cover} round="rounded-xl" />
              </span>
              {/* Pulled up so the title's capitals line up with the cover's top. */}
              <div className="-mt-[3px] flex min-w-0 flex-1 flex-col">
                <h2 className="line-clamp-3 break-words text-[20px] font-bold leading-[25px] tracking-tight text-textPrimary">
                  {song.title}
                </h2>
                <p
                  className={`mt-1.5 break-words text-[14.5px] font-medium leading-[19px] ${ink ? '' : 'text-textSecondary'}`}
                  style={ink ? { color: ink } : undefined}
                >
                  {song.artist}
                </p>
                {album && (
                  <p className="mt-1.5 flex min-w-0 items-start gap-1.5 text-[12.5px] leading-[17px] text-textMuted">
                    <Disc3 size={13} className="mt-[2px] shrink-0" aria-hidden="true" />
                    <span className="break-words">{album}</span>
                  </p>
                )}
                <div className="mt-3.5">
                  <button type="button" onClick={copyName} className={`${GLAZE_PILL} flex h-7 items-center gap-1.5 px-3 text-[12px]`}>
                    {copied ? <Check size={13} /> : <Copy size={13} />}
                    {/* Both words hold the space, so the button never changes size. */}
                    <span className="grid">
                      <span className={`col-start-1 row-start-1 ${copied ? 'invisible' : ''}`}>Copy name</span>
                      <span className={`col-start-1 row-start-1 ${copied ? '' : 'invisible'}`} aria-live="polite">
                        {copied ? 'Copied' : ''}
                      </span>
                    </span>
                  </button>
                </div>
              </div>
            </div>

            <div className="glass-tile mt-6 grid grid-cols-3 divide-x divide-borderSubtle">
              <Fact icon={Repeat} label="Times heard" value={song.count.toLocaleString('en-US')} ink={ink} />
              <Fact icon={CalendarClock} label="First heard" value={heardDay(song.first_ms, undefined, true)} ink={ink} />
              <Fact icon={History} label="Last heard" value={heardAgo(song.last_ms, undefined, true)} ink={ink} />
            </div>

            {(links.listen.length > 0 || links.search.length > 0 || links.allServices) && (
              <Section
                icon={Headphones}
                title={links.listen.length || !links.search.length ? 'Listen on' : 'Search on'}
                mark={mark}
                aside={
                  links.allServices && (
                    <Tooltip content={links.allServices} side="top" describe>
                      <button
                        type="button"
                        onClick={() => void openSongLink(links.allServices!)}
                        className="ml-auto flex h-7 items-center gap-1 text-[12px] font-medium text-textMuted transition-colors hover:text-textPrimary"
                      >
                        <Link2 size={13} aria-hidden="true" />
                        All services
                      </button>
                    </Tooltip>
                  )
                }
              >
                <SongLinkButtons links={links} title={song.title} artist={song.artist} screen={screen} />
              </Section>
            )}

            {places.length > 0 && (
              <Section icon={RadioTower} title="Where you heard it" mark={mark}>
                <div className="glass-tile divide-y divide-borderSubtle overflow-hidden">
                  {places.map((p) => (
                    <Place key={p.at} at={p.at} source={p.source} mixed={mixed} />
                  ))}
                </div>
              </Section>
            )}

            <div className="mt-6 flex justify-center">
              <button
                type="button"
                onClick={remove}
                className="flex h-8 items-center gap-1.5 rounded-full px-3.5 text-[12.5px] font-medium text-textMuted transition-colors hover:text-red-400"
              >
                <Trash2 size={14} />
                Remove from Your songs
              </button>
            </div>
          </div>
          <EdgeFade
            edge="top"
            shown={scrolledDown}
            base={baseUnder(screen, surface.h, bodyTop)}
            wash={washAt(bodyTop)}
            fadeIn={late}
          />
          <EdgeFade
            edge="bottom"
            base={baseUnder(screen, surface.h, bodyBottom - FADE_PX)}
            wash={washAt(bodyBottom - FADE_PX)}
            fadeIn={late}
          />
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- the panel

export interface SongsPanelProps {
  variant: 'modal' | 'screen';
  /** The song open in its detail, by key; null shows the list. */
  focus: string | null;
  onFocus: (key: string | null) => void;
  onClose: () => void;
  /** The modal's Escape hands the panel a turn first: the handler clears a
   *  typed search and answers true, or answers false when there is none. */
  registerEscape?: (handler: (() => boolean) | null) => void;
}

export default function SongsPanel({ variant, focus, onFocus, onClose, registerEscape }: SongsPanelProps) {
  const screen = variant === 'screen';
  const kind: SurfaceKind = screen ? 'screen' : 'flyout';
  const rowHeight = screen ? ROW.screen.height : ROW.modal.height;
  const version = useThemeVersion();
  // Which music apps open song links here: asked each time Your songs opens.
  useEffect(() => {
    refreshSongApps();
  }, []);
  const [draft, setDraft] = useState('');
  const [query, setQuery] = useState('');
  useEffect(() => {
    const t = window.setTimeout(() => setQuery(draft.trim()), 150);
    return () => window.clearTimeout(t);
  }, [draft]);

  const pages = useSongPages(query);
  const { matched, total, mixed, channels, groups, labels, rowAt, ensure, failed, retry, held } = pages;
  const listRef = useRef<List<RowData>>(null);
  useEffect(() => {
    listRef.current?.scrollTo(0);
  }, [query]);
  // Rows have scrolled up under the search field: the top edge fades them.
  const [scrolled, setScrolled] = useState(false);
  const scrollTop = useRef(0);

  // The panel's box (a song's wash covers all of it) and the list area's,
  // so react-window has a concrete height to virtualize.
  const [root, setRoot] = useState<HTMLDivElement | null>(null);
  const bodyRef = useRef<HTMLDivElement | null>(null);
  const [bodyEl, setBodyEl] = useState<HTMLDivElement | null>(null);
  const bodyRefs = useCallback((el: HTMLDivElement | null) => {
    bodyRef.current = el;
    setBodyEl(el);
  }, []);
  const surface = useBox(root);
  const body = useBox(bodyEl, root);
  const listHeight = body.h;

  // The Latest card shows the newest song when nothing is searched; then the
  // songs in their time groups, every item's height known up front.
  const layout = useMemo(
    () => makeLayout(matched ?? 0, groups, labels, !query, rowHeight),
    [matched, groups, labels, query, rowHeight],
  );
  // A new layout drops the list's kept sizes. With the same songs (the day
  // moved on and the headers changed), the song at the top stays in place.
  const shownLayout = useRef<{ layout: Layout; matched: number | null; query: string } | null>(null);
  useLayoutEffect(() => {
    const was = shownLayout.current;
    shownLayout.current = { layout, matched, query };
    const list = listRef.current;
    if (!list || !was || was.layout === layout) return;
    list.resetAfterIndex(0, true);
    if (was.matched !== matched || was.query !== query || scrollTop.current <= 0) return;
    const row = rowOfItem(was.layout, was.layout.itemAt(scrollTop.current));
    const delta = anchorDelta(was.layout, layout, row);
    if (delta) list.scrollTo(Math.max(0, scrollTop.current + delta));
  }, [layout, matched, query]);

  // On the desktop the search field takes the keyboard once there are songs
  // to search (the phone keeps its keyboard down until you tap the field).
  const searchRef = useRef<HTMLInputElement>(null);
  const hasSongs = total > 0;
  useEffect(() => {
    if (!screen && hasSongs) searchRef.current?.focus({ preventScroll: true });
  }, [screen, hasSongs]);

  useEffect(() => {
    if (!registerEscape) return;
    registerEscape(() => {
      if (!draft) return false;
      setDraft('');
      return true;
    });
    return () => registerEscape(null);
  }, [registerEscape, draft]);

  const open = useCallback((key: string) => onFocus(key), [onFocus]);
  const back = useCallback(() => onFocus(null), [onFocus]);
  const itemData = useMemo<RowData>(
    () => ({ layout, rowAt, open, mixed, screen, kind, version }),
    [layout, rowAt, open, mixed, screen, kind, version],
  );

  // Arrow keys walk the songs (the Latest card is the first; headers are
  // passed over): Down from the search to the first song, Up and Down between
  // songs, Up from the first back to the search.
  const focusRow = useCallback(
    (row: number) => {
      const find = () => bodyRef.current?.querySelector<HTMLElement>(`[data-song-row="${row}"]`);
      const rendered = find();
      if (rendered) {
        rendered.focus();
        return;
      }
      listRef.current?.scrollToItem(layout.itemOfRow(row));
      requestAnimationFrame(() => requestAnimationFrame(() => find()?.focus()));
    },
    [layout],
  );
  const onListKeyDown = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    const at = Number((e.target as HTMLElement).dataset.songRow);
    if (!Number.isInteger(at) || !matched) return;
    e.preventDefault();
    if (e.key === 'ArrowUp' && at === 0) {
      searchRef.current?.focus();
      return;
    }
    focusRow(Math.max(0, Math.min(matched - 1, e.key === 'ArrowDown' ? at + 1 : at - 1)));
  };
  const onSearchKeyDown = (e: ReactKeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'ArrowDown' && matched) {
      e.preventDefault();
      focusRow(0);
    } else if (e.key === 'Enter') {
      // The phone's search key puts the keyboard away; the desktop's Enter
      // opens the first song.
      if (screen) e.currentTarget.blur();
      else {
        const first = rowAt(0);
        if (first) open(first.key);
      }
    }
  };

  // Back from a song's detail gives the keyboard to its row again (desktop).
  const shownBefore = useRef(focus);
  useEffect(() => {
    const was = shownBefore.current;
    shownBefore.current = focus;
    if (screen || focus || !was) return;
    requestAnimationFrame(() => {
      const row = bodyRef.current?.querySelector<HTMLElement>(`[data-song-key="${CSS.escape(was)}"]`);
      (row ?? searchRef.current)?.focus({ preventScroll: true });
    });
  }, [focus, screen]);

  // A song filed while the list is scrolled waits behind the pill: pressing it
  // goes to the top, where it shows.
  const { setScrolled: setListScrolled } = pages;
  const showNew = () => {
    listRef.current?.scrollTo(0);
    scrollTop.current = 0;
    setScrolled(false);
    setListScrolled(false);
  };

  // Clear all asks once in place: the button becomes "Clear N songs" for 4 s.
  const [confirmClear, setConfirmClear] = useState(false);
  useEffect(() => {
    if (!confirmClear) return;
    const t = window.setTimeout(() => setConfirmClear(false), 4000);
    return () => window.clearTimeout(t);
  }, [confirmClear]);
  const clearAll = () => {
    if (!confirmClear) {
      setConfirmClear(true);
      return;
    }
    setConfirmClear(false);
    clearSongHistory().catch((e: unknown) => {
      Logger.warn('[Songs] clear failed:', e);
      useAppStore.getState().addToast('Could not clear Your songs', 'error');
    });
  };

  // What the list holds, under its name. Held to one line from the first
  // paint, so nothing below it moves when the count arrives.
  const summary =
    matched === null
      ? ' '
      : total > 0
        ? channels > 0
          ? `${songsCount(total)} · ${plural(channels, 'channel', 'channels')}`
          : songsCount(total)
        : 'No songs yet';

  return (
    <div ref={setRoot} className="relative flex h-full min-h-0 flex-1 flex-col">
      {/* A song's page slides in over the list, which fades back; Back plays
          it in reverse. Opacity and transform only; the Motion setting keeps
          just the fades. */}
      <motion.div
        className="flex min-h-0 flex-1 flex-col"
        initial={false}
        animate={
          focus
            ? { opacity: 0, x: -12, transitionEnd: { visibility: 'hidden' } }
            : { opacity: 1, x: 0, visibility: 'visible' }
        }
        transition={{ duration: 0.22, ease: EASE }}
      >
        <div className={screen ? 'flex h-14 shrink-0 items-center gap-2 pl-1 pr-[2.5px]' : 'flex shrink-0 items-start gap-3 px-5 pb-4 pt-5'}>
          {screen && (
            <button type="button" onClick={onClose} className="sn-touch -mr-1 flex items-center justify-center text-textSecondary" aria-label="Back">
              <PhoneArrowLeft size={22} />
            </button>
          )}
          <span className="flex min-w-0 flex-1 flex-col">
            <span
              className={`text-textPrimary ${screen ? 'text-[21px] leading-[25px]' : 'text-[24px] leading-[28px]'}`}
              style={HEADING}
            >
              {screen ? 'Songs' : 'Your songs'}
            </span>
            <span className={`truncate text-[12.5px] font-medium leading-[17px] text-textMuted ${screen ? '' : 'mt-0.5'}`}>
              {summary}
            </span>
          </span>
          {/* The actions sit on the title's cap-height midpoint, a step quieter. */}
          <div className={`flex items-center gap-3 self-start ${screen ? 'mt-[2px]' : '-mt-[2px]'}`}>
            {total > 0 &&
              (confirmClear ? (
                <button type="button" onClick={clearAll} className={`${ACCENT_BUTTON} h-8 px-3.5 text-[12.5px]`} style={DANGER_FILL}>
                  {`Clear ${songsCount(total)}`}
                </button>
              ) : screen ? (
                <button
                  type="button"
                  onClick={clearAll}
                  aria-label="Clear all"
                  className="-mr-1.5 flex h-9 w-9 items-center justify-center text-textMuted active:text-textPrimary"
                >
                  <PhoneTrash size={18} />
                </button>
              ) : (
                <Tooltip content="Clear all" side="bottom">
                  <button
                    type="button"
                    onClick={clearAll}
                    aria-label="Clear all"
                    className="flex h-8 w-8 items-center justify-center rounded-full text-textMuted transition-colors hover:text-textPrimary"
                  >
                    <Trash2 size={15} />
                  </button>
                </Tooltip>
              ))}
            {!screen && <CloseButton onClose={onClose} />}
          </div>
        </div>

        {total > 0 && (
          <div className={`shrink-0 ${screen ? 'px-4 pb-2 pt-1' : 'px-5 pb-2.5'}`}>
            <label className="glass-input flex h-10 items-center gap-2.5 rounded-xl px-3.5">
              <Search size={15} className="shrink-0 text-textMuted" />
              <input
                ref={searchRef}
                type="text"
                inputMode="search"
                enterKeyHint="search"
                autoComplete="off"
                spellCheck={false}
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={onSearchKeyDown}
                placeholder="Search songs, artists, channels or categories"
                aria-label="Search your songs"
                className="min-w-0 flex-1 bg-transparent text-[13.5px] text-textPrimary outline-none placeholder:text-textMuted"
              />
              {draft && (
                <button
                  type="button"
                  onClick={() => {
                    setDraft('');
                    searchRef.current?.focus();
                  }}
                  aria-label="Clear search"
                  className="-mr-1 flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-textMuted transition-colors hover:text-textPrimary"
                >
                  <X size={14} />
                </button>
              )}
            </label>
          </div>
        )}

        <div
          ref={bodyRefs}
          className="relative min-h-0 flex-1"
          data-taps-through-keyboard=""
          onKeyDown={onListKeyDown}
          style={screen ? { paddingBottom: 'var(--sn-safe-b, 0px)' } : undefined}
        >
          {failed && matched === null ? (
            <LoadFailed what="Couldn't open your songs." onRetry={retry} />
          ) : matched === null ? null : matched === 0 ? (
            <EmptyState searched={total > 0 && query ? query : null} />
          ) : (
            listHeight > 0 && (
              <List<RowData>
                ref={listRef}
                className="sn-songs-scroll"
                height={listHeight}
                width="100%"
                itemCount={layout.count}
                itemSize={layout.size}
                estimatedItemSize={rowHeight}
                itemData={itemData}
                itemKey={(index, data) => {
                  const item = data.layout.item(index);
                  if (item.kind === 'latest') return 'latest';
                  if (item.kind === 'header') return `group-${item.label}`;
                  return data.rowAt(item.row)?.key ?? `pending-${item.row}`;
                }}
                overscanCount={6}
                onItemsRendered={({ overscanStartIndex, overscanStopIndex }) =>
                  ensure(rowOfItem(layout, overscanStartIndex), rowOfItem(layout, overscanStopIndex))
                }
                onScroll={({ scrollOffset }) => {
                  scrollTop.current = scrollOffset;
                  setScrolled(scrollOffset > 0);
                  setListScrolled(scrollOffset > 0);
                }}
              >
                {ListItem}
              </List>
            )
          )}
          {matched ? (
            <>
              <EdgeFade edge="top" base={baseUnder(screen, surface.h, body.y)} shown={scrolled} />
              <EdgeFade
                edge="bottom"
                base={baseUnder(screen, surface.h, body.y + body.h - FADE_PX)}
                offset={screen ? 'var(--sn-safe-b, 0px)' : undefined}
              />
            </>
          ) : null}
          {/* The "New song" pill drops in from the top of the list. */}
          <div className="pointer-events-none absolute inset-x-0 top-2 z-10 flex justify-center">
            <AnimatePresence>
              {held > 0 && (
                <motion.button
                  key="new-song"
                  type="button"
                  onClick={showNew}
                  initial={{ opacity: 0, y: -10 }}
                  animate={{ opacity: 1, y: 0, transition: { duration: 0.2, ease: EASE } }}
                  exit={{ opacity: 0, y: -8, transition: { duration: 0.14, ease: EASE } }}
                  className="pointer-events-auto flex h-8 items-center gap-1.5 whitespace-nowrap px-3.5 text-[12.5px] font-medium text-white chrome-glaze chrome-glaze--control chrome-glaze--frosted"
                >
                  <ArrowUp size={14} strokeWidth={2.3} aria-hidden="true" />
                  {held === 1 ? 'New song' : `${held.toLocaleString('en-US')} new songs`}
                </motion.button>
              )}
            </AnimatePresence>
          </div>
        </div>
      </motion.div>

      <AnimatePresence>
        {focus && (
          <motion.div
            key={focus}
            className="absolute inset-0"
            initial={{ opacity: 0, x: 24 }}
            animate={{ opacity: 1, x: 0, transition: { duration: 0.22, ease: EASE } }}
            exit={{ opacity: 0, x: 24, transition: { duration: 0.18, ease: EASE } }}
          >
            <SongDetail
              songKey={focus}
              mixed={mixed}
              screen={screen}
              root={root}
              surface={surface}
              onBack={back}
              onClose={onClose}
            />
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
