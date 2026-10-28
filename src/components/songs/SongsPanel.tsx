// Your songs: every song you identified, newest heard first. One body for both
// shells, the desktop's modal (SongsPanelHost) and the phone's You > Songs
// screen (mobile/screens/SongsScreen). Rust keeps, searches and pages the songs
// (services/song_id/history.rs); this draws a window of rows and one song's
// detail over the list, which stays mounted so it keeps its scroll.
import {
  memo,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
} from 'react';
import { FixedSizeList as List, type ListChildComponentProps } from 'react-window';
import {
  ArrowLeft,
  AudioLines,
  CalendarClock,
  Check,
  Copy,
  Disc3,
  Headphones,
  History,
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
import { AUDIO_GRAPH_SUPPORTED } from '../../utils/audioBoost';
import { Logger } from '../../utils/logger';
import { openExternal } from '../../utils/openExternal';
import { IS_MOBILE } from '../../utils/platform';
import {
  clearSongHistory,
  fetchSong,
  onSongHistoryChanged,
  removeSong,
  restoreSong,
  type SongEntry,
  type SongRow,
  type SongSource,
} from '../../utils/songHistory';
import { channelOf, heardAgo, heardClock, heardDay, heardInto, mediaMark } from '../../utils/songWords';
import { ProviderLogo } from '../ProviderLogo';
import { SocialLogo } from '../ui/SocialLogo';
import { Tooltip } from '../ui/Tooltip';
import { ACCENT_BUTTON, DANGER_FILL, GLAZE_PILL } from '../ui/glazeButtons';
import { markOf } from './serviceMarks';
import { useSongPages } from './useSongPages';

/** Row height, art size and type for each shell: the phone's rows are roomier,
 *  like the You screen's. */
const ROW = {
  modal: { height: 76, art: 52, title: 'text-[14px]', artist: 'text-[12.5px]', heard: 'text-[12px]' },
  screen: { height: 80, art: 56, title: 'text-[15px]', artist: 'text-[13px]', heard: 'text-[12.5px]' },
} as const;

const plural = (n: number, one: string, many: string) => (n === 1 ? `1 ${one}` : `${n.toLocaleString('en-US')} ${many}`);
const songsCount = (n: number) => plural(n, 'song', 'songs');

/** The accent's own tint, for the marks that head the panel and its sections. */
const MARK_STYLE = {
  background:
    'linear-gradient(150deg, color-mix(in srgb, var(--color-accent) 34%, transparent), color-mix(in srgb, var(--color-accent) 12%, transparent))',
  boxShadow:
    'inset 0 1px 0 rgba(255, 255, 255, 0.14), inset 0 0 0 1px color-mix(in srgb, var(--color-accent) 28%, transparent)',
} as const;

/** Open a song's link, and say so when the system would not open it. */
function openLink(url: string): void {
  void openExternal(url).then((opened) => {
    if (!opened) useAppStore.getState().addToast("Couldn't open that link", 'error');
  });
}

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
        className={`flex shrink-0 items-center justify-center bg-white/[0.06] ${round}`}
        style={{ width: px, height: px }}
        aria-hidden="true"
      >
        <Music size={Math.round(px * 0.38)} className="text-white/35" />
      </div>
    );
  }
  return (
    <span className={`relative block shrink-0 bg-white/[0.06] ${round}`} style={{ width: px, height: px }}>
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
        className="flex shrink-0 items-center justify-center rounded-full bg-white/10 font-semibold leading-none text-textSecondary"
        style={{ width: px, height: px, fontSize: Math.max(8, Math.round(px * 0.46)) }}
        aria-hidden="true"
      >
        {name.charAt(0).toUpperCase()}
      </span>
    );
  }
  return (
    <span className="relative block shrink-0 rounded-full bg-white/10" style={{ width: px, height: px }} aria-hidden="true">
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
    <span className="shrink-0 rounded-[5px] bg-white/[0.08] px-1.5 py-px text-[10.5px] font-semibold leading-[15px] text-textSecondary">
      {mark}
    </span>
  );
}

/** The accent mark at the head of the panel and of an empty list. */
function PanelMark({ px, icon: Icon = AudioLines }: { px: number; icon?: LucideIcon }) {
  return (
    <span
      className="flex shrink-0 items-center justify-center text-accent-on-surface"
      style={{ ...MARK_STYLE, width: px, height: px, borderRadius: Math.round(px * 0.3) }}
      aria-hidden="true"
    >
      <Icon size={Math.round(px * 0.5)} strokeWidth={2.1} />
    </span>
  );
}

interface RowData {
  rowAt: (index: number) => SongRow | null;
  open: (key: string) => void;
  /** The list mixes platforms: each place carries its platform's logo. */
  mixed: boolean;
  screen: boolean;
}

const SongListRow = memo(function SongListRow({ index, style, data }: ListChildComponentProps<RowData>) {
  const size = data.screen ? ROW.screen : ROW.modal;
  const row = data.rowAt(index);
  if (!row) {
    return (
      <div style={style} className="pl-2" aria-hidden="true">
        <div className="flex h-full items-center gap-3.5 px-2.5">
          <div className="shrink-0 rounded-lg bg-white/[0.05]" style={{ width: size.art, height: size.art }} />
          <div className="flex min-w-0 flex-1 flex-col gap-2">
            <div className="h-3 w-2/5 rounded bg-white/[0.06]" />
            <div className="h-2.5 w-1/3 rounded bg-white/[0.04]" />
            <div className="h-2.5 w-1/2 rounded bg-white/[0.04]" />
          </div>
        </div>
      </div>
    );
  }
  const heard = row.heard;
  return (
    <div style={style} className="pl-2">
      <button
        type="button"
        data-song-row={index}
        data-song-key={row.key}
        onClick={() => data.open(row.key)}
        className="flex h-full w-full items-center gap-3.5 rounded-xl px-2.5 text-left outline-none transition-colors duration-150 hover:bg-glass-hover focus-visible:bg-glass-hover"
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
          {heard && (
            <span className={`flex min-w-0 items-center gap-1.5 leading-[17px] text-textMuted ${size.heard}`}>
              <ChannelFace key={heard.avatar ?? ''} src={heard.avatar} name={channelOf(heard)} px={15} />
              {/* One run of text, so a cut takes the category before the channel. */}
              <span className="min-w-0 truncate">
                <span className="text-textSecondary">{channelOf(heard)}</span>
                {heard.category && <span> · {heard.category}</span>}
              </span>
              {data.mixed && <ProviderLogo provider={heard.provider} size={11} className="shrink-0" />}
              <MediaChip source={heard} />
              {row.count > 1 && (
                <span className="ml-auto flex shrink-0 items-center gap-1 pl-2 text-[11px] font-medium tabular-nums">
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
});

function EmptyState({ searched }: { searched: string | null }) {
  if (searched) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 px-8 pb-10 text-center">
        <span className="flex h-12 w-12 items-center justify-center rounded-2xl bg-white/[0.06] text-textMuted" aria-hidden="true">
          <Search size={22} />
        </span>
        <div className="flex flex-col gap-1">
          <p className="text-[14px] font-semibold text-textPrimary">No songs match “{searched}”</p>
          <p className="text-[12.5px] text-textMuted">Search by song, artist, channel or category.</p>
        </div>
      </div>
    );
  }
  return (
    <div className="flex h-full flex-col items-center justify-center gap-4 px-8 pb-10 text-center">
      <PanelMark px={60} />
      <div className="flex max-w-[280px] flex-col gap-1.5">
        <p className="text-[15px] font-semibold text-textPrimary">Songs you identify land here</p>
        {AUDIO_GRAPH_SUPPORTED && (
          <p className="text-[12.5px] leading-[19px] text-textMuted">
            {IS_MOBILE ? 'Type ' : 'Press the music note on the player, or type '}
            <kbd className="rounded-md border border-white/10 bg-white/[0.08] px-1.5 py-px font-mono text-[11.5px] text-textSecondary">
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
        <X size={18} />
      </button>
    </Tooltip>
  );
}

/** A song's top bar. The phone keeps the list's own header row and arrow (the
 *  song's title below names the page); the modal keeps its Close where the
 *  list has it. */
function BackBar({ screen, onBack, onClose }: { screen: boolean; onBack: () => void; onClose: () => void }) {
  if (screen) {
    return (
      <div className="flex h-10 shrink-0 items-center pl-1 pr-[2.5px]">
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
    <div className="flex shrink-0 items-center gap-2 pb-2 pl-2 pr-4 pt-4">
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

/** A part of a song's page: its accent mark and name over what it holds. */
function Section({ icon: Icon, title, children }: { icon: LucideIcon; title: string; children: ReactNode }) {
  return (
    <section className="mt-6">
      <div className="mb-2.5 flex items-center gap-2">
        <span
          className="flex h-[22px] w-[22px] shrink-0 items-center justify-center rounded-[7px] text-accent-on-surface"
          style={MARK_STYLE}
          aria-hidden="true"
        >
          <Icon size={12.5} strokeWidth={2.3} />
        </span>
        <h3 className="text-[13px] font-semibold text-textPrimary">{title}</h3>
      </div>
      {children}
    </section>
  );
}

/** One fact in the song's facts card: its name over its value. */
function Fact({ icon: Icon, label, value }: { icon: LucideIcon; label: string; value: string }) {
  return (
    <div className="flex min-w-0 flex-col gap-1 px-3 py-2.5">
      <span className="flex items-center gap-1.5 text-[11px] font-medium text-textMuted">
        <Icon size={12} strokeWidth={2.2} aria-hidden="true" className="shrink-0" />
        <span className="truncate">{label}</span>
      </span>
      <span className="whitespace-nowrap text-[14px] font-semibold tabular-nums text-textPrimary">{value}</span>
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
  onBack,
  onClose,
}: {
  songKey: string;
  mixed: boolean;
  screen: boolean;
  onBack: () => void;
  onClose: () => void;
}) {
  // undefined while loading, null once it is gone (removed, cleared, never kept).
  const [song, setSong] = useState<SongEntry | null | undefined>(undefined);
  const [failed, setFailed] = useState(false);
  // Bumped by Try again, which asks once more.
  const [attempt, setAttempt] = useState(0);
  const [copied, setCopied] = useState(false);

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
    const unlisten = onSongHistoryChanged((change) => {
      if (change.kind === 'cleared' || (change.kind === 'removed' && change.key === songKey)) {
        asked += 1;
        if (alive) setSong(null);
      } else if (change.kind === 'filed' && change.key === songKey) {
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

  if (failed && song === undefined) {
    return (
      <div className="absolute inset-0 flex min-h-0 flex-col">
        <BackBar screen={screen} onBack={onBack} onClose={onClose} />
        <LoadFailed what="Couldn't open this song." onRetry={() => setAttempt((n) => n + 1)} />
      </div>
    );
  }
  if (!song) return null;

  const links = [...song.providers];
  if (song.shazam_url) links.push({ name: 'Shazam', url: song.shazam_url });
  if (song.song_link) links.push({ name: 'song.link', url: song.song_link });
  const places = song.sightings.flatMap((s) => (s.source ? [{ at: s.at_ms, source: s.source }] : []));
  const cover = screen ? 116 : 128;

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
      <BackBar screen={screen} onBack={onBack} onClose={onClose} />
      <div
        className={`custom-scrollbar min-h-0 flex-1 overflow-y-auto ${screen ? 'px-4' : 'px-5 pb-6'}`}
        style={screen ? { paddingBottom: 'calc(var(--sn-safe-b, 0px) + 24px)' } : undefined}
      >
        <div className="flex items-start gap-5 pt-2">
          {/* The cover lights the panel under it with its own colors, a soft
              glow below and around it that never reaches the words. */}
          <div className="relative shrink-0" style={{ width: cover, height: cover }}>
            {song.album_art && (
              <img
                src={song.album_art}
                alt=""
                aria-hidden="true"
                className="pointer-events-none absolute left-[8%] top-[14%] h-[90%] w-[84%] rounded-xl object-cover opacity-55 blur-xl saturate-150"
              />
            )}
            <span className="relative block rounded-xl shadow-[0_12px_28px_-14px_rgba(0,0,0,0.8)] ring-1 ring-inset ring-white/10">
              <SongArt key={song.album_art ?? ''} src={song.album_art} px={cover} round="rounded-xl" />
            </span>
          </div>
          <div className="flex min-w-0 flex-1 flex-col pt-1">
            <h2 className="line-clamp-3 break-words text-[20px] font-bold leading-[25px] tracking-tight text-textPrimary">
              {song.title}
            </h2>
            <p className="mt-1.5 break-words text-[14.5px] font-medium leading-[19px] text-textSecondary">{song.artist}</p>
            {song.album && (
              <p className="mt-1.5 flex min-w-0 items-start gap-1.5 text-[12.5px] leading-[17px] text-textMuted">
                <Disc3 size={13} className="mt-[2px] shrink-0" aria-hidden="true" />
                <span className="break-words">{song.album}</span>
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

        <div className="glass-tile mt-6 grid grid-cols-3 divide-x divide-white/[0.06]">
          <Fact icon={Repeat} label="Times heard" value={song.count.toLocaleString('en-US')} />
          <Fact icon={CalendarClock} label="First heard" value={heardDay(song.first_ms)} />
          <Fact icon={History} label="Last heard" value={heardAgo(song.last_ms)} />
        </div>

        {links.length > 0 && (
          <Section icon={Headphones} title={song.links_are_searches ? 'Find it on' : 'Listen on'}>
            <div className="flex flex-wrap gap-2">
              {links.map((l) => (
                <Tooltip key={l.url} content={l.url} side="top" describe>
                  <button
                    type="button"
                    onClick={() => openLink(l.url)}
                    className={`${GLAZE_PILL} flex h-9 items-center gap-2 pl-2.5 pr-3.5 text-[12.5px]`}
                  >
                    <SocialLogo platform={markOf(l.name)} size={17} />
                    {l.name}
                  </button>
                </Tooltip>
              ))}
            </div>
          </Section>
        )}

        {places.length > 0 && (
          <Section icon={RadioTower} title="Where you heard it">
            <div className="glass-tile divide-y divide-white/[0.06] overflow-hidden">
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
    </div>
  );
}

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
  const [draft, setDraft] = useState('');
  const [query, setQuery] = useState('');
  useEffect(() => {
    const t = window.setTimeout(() => setQuery(draft.trim()), 150);
    return () => window.clearTimeout(t);
  }, [draft]);

  const { matched, total, mixed, channels, rowAt, ensure, failed, retry } = useSongPages(query);
  const listRef = useRef<List<RowData>>(null);
  useEffect(() => {
    listRef.current?.scrollTo(0);
  }, [query]);

  // Measure the list area so react-window has a concrete height to virtualize.
  const bodyRef = useRef<HTMLDivElement>(null);
  const [listHeight, setListHeight] = useState(0);
  useEffect(() => {
    const el = bodyRef.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) setListHeight(entry.contentRect.height);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

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
  const itemData = useMemo<RowData>(() => ({ rowAt, open, mixed, screen }), [rowAt, open, mixed, screen]);

  // Arrow keys walk the list: Down from the search to the first song, Up and
  // Down between songs, Up from the first back to the search.
  const focusRow = useCallback((index: number) => {
    const find = () => bodyRef.current?.querySelector<HTMLElement>(`[data-song-row="${index}"]`);
    const rendered = find();
    if (rendered) {
      rendered.focus();
      return;
    }
    listRef.current?.scrollToItem(index);
    requestAnimationFrame(() => requestAnimationFrame(() => find()?.focus()));
  }, []);
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
      ? ' '
      : total > 0
        ? channels > 0
          ? `${songsCount(total)} · ${plural(channels, 'channel', 'channels')}`
          : songsCount(total)
        : 'No songs yet';

  return (
    <div className="relative flex h-full min-h-0 flex-1 flex-col">
      <div className={`flex min-h-0 flex-1 flex-col ${focus ? 'invisible' : ''}`}>
        <div className={screen ? 'flex h-14 shrink-0 items-center gap-2 pl-1 pr-[2.5px]' : 'flex shrink-0 items-center gap-3 px-4 pb-3.5 pt-4'}>
          {screen && (
            <button type="button" onClick={onClose} className="sn-touch -mr-1 flex items-center justify-center text-textSecondary" aria-label="Back">
              <PhoneArrowLeft size={22} />
            </button>
          )}
          <PanelMark px={screen ? 34 : 38} />
          <span className="flex min-w-0 flex-1 flex-col">
            <span className={`font-bold leading-[21px] tracking-tight text-textPrimary ${screen ? 'text-[17px]' : 'text-[16px]'}`}>
              {screen ? 'Songs' : 'Your songs'}
            </span>
            <span className="truncate text-[12px] font-medium leading-[16px] tabular-nums text-textMuted">{summary}</span>
          </span>
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
                  <Trash2 size={16} />
                </button>
              </Tooltip>
            ))}
          {!screen && <CloseButton onClose={onClose} />}
        </div>

        {total > 0 && (
          <div className={`shrink-0 px-4 ${screen ? 'pb-2 pt-1' : 'pb-2'}`}>
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
          ref={bodyRef}
          className="min-h-0 flex-1"
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
                className="custom-scrollbar"
                style={{ scrollbarGutter: 'stable' }}
                height={listHeight}
                width="100%"
                itemCount={matched}
                itemSize={screen ? ROW.screen.height : ROW.modal.height}
                itemData={itemData}
                itemKey={(index, data) => data.rowAt(index)?.key ?? `pending-${index}`}
                overscanCount={6}
                onItemsRendered={({ overscanStartIndex, overscanStopIndex }) => ensure(overscanStartIndex, overscanStopIndex)}
              >
                {SongListRow}
              </List>
            )
          )}
        </div>
      </div>

      {focus && <SongDetail key={focus} songKey={focus} mixed={mixed} screen={screen} onBack={back} onClose={onClose} />}
    </div>
  );
}
