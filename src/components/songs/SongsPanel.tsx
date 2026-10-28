// Your songs: every song you identified, newest heard first. One body for both
// shells, the desktop's modal (SongsPanelHost) and the phone's You > Songs
// screen (mobile/screens/SongsScreen). Rust keeps, searches and pages the songs
// (services/song_id/history.rs); this draws a window of rows and one song's
// detail over the list, which stays mounted so it keeps its scroll.
import { memo, useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from 'react';
import { FixedSizeList as List, type ListChildComponentProps } from 'react-window';
import { ArrowLeft, Check, Copy, Music, Search, Trash2, X } from 'lucide-react';
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
} from '../../utils/songHistory';
import { heardAgo, heardAt, placeOf } from '../../utils/songWords';
import { ProviderLogo } from '../ProviderLogo';
import { Tooltip } from '../ui/Tooltip';
import { ACCENT_BUTTON, DANGER_FILL, GLAZE_PILL } from '../ui/glazeButtons';
import { useSongPages } from './useSongPages';

const ROW_HEIGHT = 60;

/** Open a song's link, and say so when the system would not open it. */
function openLink(url: string): void {
  void openExternal(url).then((opened) => {
    if (!opened) useAppStore.getState().addToast("Couldn't open that link", 'error');
  });
}

/** A cover, or the music mark when there is none or it will not load. Tries
 *  `fallback` (the full cover) when a small copy is refused. Callers key it by
 *  `src`, so a new cover starts over. */
function SongArt({ src, fallback, px }: { src: string | null; fallback?: string | null; px: number }) {
  const [step, setStep] = useState(0);
  const url = step === 0 ? src : step === 1 ? (fallback ?? null) : null;
  if (!url) {
    return (
      <div
        className="flex shrink-0 items-center justify-center rounded-md bg-white/5"
        style={{ width: px, height: px }}
        aria-hidden="true"
      >
        <Music size={Math.round(px * 0.4)} className="text-white/40" />
      </div>
    );
  }
  return (
    <img
      src={url}
      alt=""
      width={px}
      height={px}
      loading="lazy"
      decoding="async"
      onError={() => setStep((s) => (s === 0 && fallback ? 1 : 2))}
      className="shrink-0 rounded-md object-cover"
      style={{ width: px, height: px }}
    />
  );
}

interface RowData {
  rowAt: (index: number) => SongRow | null;
  open: (key: string) => void;
  /** The list mixes platforms: each place carries its platform's logo. */
  mixed: boolean;
}

const SongListRow = memo(function SongListRow({ index, style, data }: ListChildComponentProps<RowData>) {
  const row = data.rowAt(index);
  if (!row) {
    return (
      <div style={style} className="px-2" aria-hidden="true">
        <div className="flex h-full items-center gap-3 px-2">
          <div className="h-10 w-10 shrink-0 rounded-md bg-white/5" />
          <div className="flex min-w-0 flex-1 flex-col gap-1.5">
            <div className="h-3 w-2/5 rounded bg-white/5" />
            <div className="h-2.5 w-3/5 rounded bg-white/[0.04]" />
          </div>
        </div>
      </div>
    );
  }
  return (
    <div style={style} className="px-2">
      <button
        type="button"
        data-song-row={index}
        data-song-key={row.key}
        onClick={() => data.open(row.key)}
        className="flex h-full w-full items-center gap-3 rounded-lg px-2 text-left outline-none transition-colors duration-150 hover:bg-glass-hover focus-visible:bg-glass-hover"
      >
        <SongArt key={row.art ?? ''} src={row.art} fallback={row.art_full} px={40} />
        <span className="min-w-0 flex-1">
          <span className="block truncate text-[13.5px] font-semibold text-textPrimary">{row.title}</span>
          <span className="flex min-w-0 items-center gap-1 text-[12px] text-textMuted">
            <span className="truncate">{row.artist}</span>
            {row.heard && (
              <>
                <span aria-hidden="true">·</span>
                {data.mixed && <ProviderLogo provider={row.heard.provider} size={11} />}
                <span className="truncate">{placeOf(row.heard)}</span>
              </>
            )}
            {row.count > 1 && (
              <>
                <span aria-hidden="true">·</span>
                <span className="shrink-0">{row.count} times</span>
              </>
            )}
          </span>
        </span>
        <span className="shrink-0 self-start pt-[12px] text-[11px] tabular-nums text-textMuted">
          {heardAgo(row.last_ms)}
        </span>
      </button>
    </div>
  );
});

function EmptyState({ searched }: { searched: string | null }) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 px-8 text-center">
      <Music size={28} className="text-white/25" />
      {searched ? (
        <p className="text-[13.5px] text-textSecondary">No songs match “{searched}”.</p>
      ) : (
        <>
          <p className="text-[13.5px] font-medium text-textPrimary">Songs you identify land here</p>
          {AUDIO_GRAPH_SUPPORTED && (
            <p className="text-[12.5px] text-textMuted">
              {IS_MOBILE
                ? 'Type /song in a chat while a stream plays.'
                : 'Press the music note on the player, or type /song in chat.'}
            </p>
          )}
        </>
      )}
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

function BackBar({ onBack }: { onBack: () => void }) {
  return (
    <div className="flex h-12 shrink-0 items-center px-2">
      <button
        type="button"
        onClick={onBack}
        className="flex h-9 items-center gap-1.5 rounded-lg px-2 text-[13px] font-medium text-textSecondary transition-colors hover:bg-glass-hover hover:text-textPrimary"
      >
        <ArrowLeft size={18} />
        Your songs
      </button>
    </div>
  );
}

function SongDetail({
  songKey,
  mixed,
  screen,
  onBack,
}: {
  songKey: string;
  mixed: boolean;
  screen: boolean;
  onBack: () => void;
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
        <BackBar onBack={onBack} />
        <LoadFailed what="Couldn't open this song." onRetry={() => setAttempt((n) => n + 1)} />
      </div>
    );
  }
  if (!song) return null;

  const links = [...song.providers];
  if (song.song_link) links.push({ name: 'song.link', url: song.song_link });
  const places = song.sightings.flatMap((s) => (s.source ? [{ at: s.at_ms, source: s.source }] : []));

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
      <BackBar onBack={onBack} />
      <div
        className={`custom-scrollbar min-h-0 flex-1 overflow-y-auto px-5 ${screen ? '' : 'pb-5'}`}
        style={screen ? { paddingBottom: 'calc(var(--sn-safe-b, 0px) + 20px)' } : undefined}
      >
        <div className="flex items-start gap-4">
          <SongArt key={song.album_art ?? ''} src={song.album_art} px={112} />
          <div className="min-w-0 flex-1 pt-1">
            <h2 className="break-words text-[18px] font-bold leading-tight text-textPrimary">{song.title}</h2>
            <p className="mt-1 break-words text-[14px] text-textSecondary">{song.artist}</p>
            {song.album && <p className="mt-0.5 break-words text-[12.5px] text-textMuted">{song.album}</p>}
          </div>
        </div>

        {links.length > 0 && (
          <div className="mt-4 flex flex-wrap items-center gap-x-4 gap-y-1.5">
            {links.map((l) => (
              <Tooltip key={l.url} content={l.url} side="top">
                <button
                  type="button"
                  onClick={() => openLink(l.url)}
                  className="text-[13px] font-medium text-textSecondary transition-colors duration-150 hover:text-accent"
                >
                  {l.name}
                </button>
              </Tooltip>
            ))}
          </div>
        )}

        <div className="mt-4 flex items-center gap-2">
          <button type="button" onClick={copyName} className={`${GLAZE_PILL} flex h-8 items-center gap-1.5 px-3.5 text-[12.5px]`}>
            {copied ? <Check size={14} /> : <Copy size={14} />}
            {/* Both words hold the space, so the row never shifts when the label changes. */}
            <span className="grid">
              <span className={`col-start-1 row-start-1 ${copied ? 'invisible' : ''}`}>Copy name</span>
              <span className={`col-start-1 row-start-1 ${copied ? '' : 'invisible'}`} aria-live="polite">
                {copied ? 'Copied' : ''}
              </span>
            </span>
          </button>
          <button
            type="button"
            onClick={remove}
            className="flex h-8 items-center gap-1.5 rounded-full px-3 text-[12.5px] font-medium text-textMuted transition-colors hover:text-red-400"
          >
            <Trash2 size={14} />
            Remove
          </button>
        </div>

        {places.length > 0 && (
          <div className="mt-6">
            <div className="mb-1.5 text-[11px] font-semibold uppercase tracking-wide text-textMuted">
              {song.count > 1 ? `Heard ${song.count} times` : 'Heard on'}
            </div>
            <div className="flex flex-col">
              {places.map((p) => (
                <div key={p.at} className="flex items-baseline justify-between gap-3 py-1.5 text-[13px]">
                  <span className="flex min-w-0 items-center gap-1.5 text-textPrimary">
                    {mixed && <ProviderLogo provider={p.source.provider} size={12} />}
                    <span className="truncate">{placeOf(p.source)}</span>
                  </span>
                  <span className="shrink-0 text-[12px] tabular-nums text-textMuted">{heardAt(p.at)}</span>
                </div>
              ))}
            </div>
          </div>
        )}
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

  const { matched, total, mixed, rowAt, ensure, failed, retry } = useSongPages(query);
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
  const itemData = useMemo<RowData>(() => ({ rowAt, open, mixed }), [rowAt, open, mixed]);

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

  return (
    <div className="relative flex h-full min-h-0 flex-1 flex-col">
      <div className={`flex min-h-0 flex-1 flex-col ${focus ? 'invisible' : ''}`}>
        <div className={screen ? 'flex h-10 shrink-0 items-center pl-1 pr-[2.5px]' : 'flex shrink-0 items-center gap-2 px-4 pb-3 pt-4'}>
          {screen && (
            <button type="button" onClick={onClose} className="sn-touch -mr-1 flex items-center justify-center text-textSecondary" aria-label="Back">
              <ArrowLeft size={22} />
            </button>
          )}
          <span className="flex min-w-0 items-baseline gap-2">
            <span className="text-[16px] font-bold tracking-tight text-textPrimary">{screen ? 'Songs' : 'Your songs'}</span>
            {total > 0 && (
              <span className="text-[12.5px] font-medium tabular-nums text-textMuted">
                {total === 1 ? '1 song' : `${total} songs`}
              </span>
            )}
          </span>
          <span className="flex-1" />
          {total > 0 &&
            (confirmClear ? (
              <button type="button" onClick={clearAll} className={`${ACCENT_BUTTON} h-8 px-3.5 text-[12.5px]`} style={DANGER_FILL}>
                {total === 1 ? 'Clear 1 song' : `Clear ${total} songs`}
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
          {!screen && (
            <Tooltip content="Close" side="bottom">
              <button
                type="button"
                onClick={onClose}
                aria-label="Close"
                className="flex h-8 w-8 items-center justify-center rounded-full text-textMuted transition-colors hover:text-textPrimary"
              >
                <X size={18} />
              </button>
            </Tooltip>
          )}
        </div>

        {total > 0 && (
          <div className="shrink-0 px-4 pb-2">
            <label className="glass-input flex h-9 items-center gap-2 rounded-lg px-3">
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
                placeholder="Search by song, artist or channel"
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
                height={listHeight}
                width="100%"
                itemCount={matched}
                itemSize={ROW_HEIGHT}
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

      {focus && <SongDetail key={focus} songKey={focus} mixed={mixed} screen={screen} onBack={back} />}
    </div>
  );
}
