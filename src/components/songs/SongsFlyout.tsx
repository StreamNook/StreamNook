// The player's Songs control: one glazed split button. The note identifies the
// song playing (it reads "Stop listening", with bars that move with the
// stream's sound, while a capture runs); the small arrow beside it opens the
// Songs popover: Identify this song, the four latest songs, and All your
// songs. Opening the popover records nothing. The arrow works on every
// platform, including the one where identifying is closed (macOS), so Your
// songs is always one click from the player.
//
// The popover is portaled to the page's body with fixed placement, since a
// frosted menu inside the player's glazed cluster would only blur the cluster
// (memory: menus over video are glass-flyout, portaled out). The player keeps
// its overlay up while it is open by finding it on the document
// (songsPopover.ts songsFlyoutOpen). It opens and closes by opacity and
// transform only; the Motion setting drops the movement and keeps the fade
// (MotionScope's MotionConfig).
import './songsScroll.css';
import { useCallback, useEffect, useRef, useState, useSyncExternalStore, type CSSProperties, type RefObject } from 'react';
import { createPortal } from 'react-dom';
import { AnimatePresence, motion } from 'framer-motion';
import { ChevronDown, ChevronRight, Loader2, Music } from 'lucide-react';
import { useOutsidePress } from '../../hooks/useOutsidePress';
import { useAppStore } from '../../stores/AppStore';
import { subscribeTheme, themeVersion } from '../../themes';
import { readSurface, rowTint } from '../../utils/coverTint';
import { Logger } from '../../utils/logger';
import { fetchSongPage, onSongHistoryChanged, openSongs, type SongRow } from '../../utils/songHistory';
import { cancelListening, getListening, subscribeListening, type Listening } from '../../utils/songListening';
import { heardAgo } from '../../utils/songWords';
import { onWindowVisibility, isWindowHidden } from '../../utils/windowVisibility';
import { ListeningBars } from '../chat/ListeningStrip';
import { Tooltip } from '../ui/Tooltip';
import { ACCENT_BUTTON, ACCENT_FILL } from '../ui/glazeButtons';
import { flyoutZIndex, placeFlyout, SONGS_FLYOUT_ATTR, type Placement } from './songsPopover';

/** Recent songs the popover shows. */
const RECENT = 4;
/** The house easing for these moves. */
const EASE = [0.2, 0.8, 0.2, 1] as const;
/** The Your songs heading, in Inter (loaded app-wide). */
const HEADING: CSSProperties = {
  fontFamily: '"Inter", var(--app-font, sans-serif)',
  fontWeight: 780,
  letterSpacing: '-0.02em',
};

const noListening = (): Listening | null => null;
const serverVersion = () => 0;
const songsCount = (n: number) => (n === 1 ? '1 song' : `${n.toLocaleString('en-US')} songs`);

/** A recent song's cover, or the music mark when it has none or it will not load. */
function RecentCover({ src, fallback }: { src: string | null; fallback?: string }) {
  const [step, setStep] = useState(0);
  const url = step === 0 ? src : step === 1 ? (fallback ?? null) : null;
  if (!url) {
    return (
      <span className="flex h-12 w-12 shrink-0 items-center justify-center rounded-lg bg-glass-hover" aria-hidden="true">
        <Music size={18} className="text-textMuted" />
      </span>
    );
  }
  return (
    <img
      src={url}
      alt=""
      width={48}
      height={48}
      loading="lazy"
      decoding="async"
      onError={() => setStep((s) => (s === 0 && fallback ? 1 : 2))}
      className="h-12 w-12 shrink-0 rounded-lg bg-glass-hover object-cover"
    />
  );
}

export interface SongsFlyoutBodyProps {
  /** The latest songs, newest first. */
  recent: SongRow[];
  /** Songs in Your songs; null when the list could not be read. */
  total: number | null;
  /** Identifying works here (macOS closes the audio tap it listens through). */
  supported: boolean;
  /** Why it does not, shown in place of the hint. */
  refusal: string;
  /** A listening runs in this window. */
  listening: boolean;
  /** A recent row's hover tint (null keeps the ordinary hover). */
  tintFor: (row: SongRow) => string | null;
  onIdentify: () => void;
  onOpenSong: (key: string) => void;
  onOpenAll: () => void;
}

/** What the popover holds. */
export function SongsFlyoutBody({
  recent,
  total,
  supported,
  refusal,
  listening,
  tintFor,
  onIdentify,
  onOpenSong,
  onOpenAll,
}: SongsFlyoutBodyProps) {
  return (
    <>
      <div className="flex shrink-0 items-baseline gap-3 px-4 pb-2.5 pt-4">
        <h2 className="text-[18px] leading-[23px] text-textPrimary" style={HEADING}>
          Your songs
        </h2>
        {!!total && (
          <span className="ml-auto text-[12px] font-medium tabular-nums text-textMuted">{songsCount(total)}</span>
        )}
      </div>
      <div className="shrink-0 border-b border-borderSubtle px-3 pb-3.5">
        <button
          type="button"
          onClick={onIdentify}
          disabled={!supported || listening}
          className={`${ACCENT_BUTTON} flex h-12 w-full items-center justify-center gap-2.5 !text-[14px]`}
          style={ACCENT_FILL}
        >
          {listening ? <ListeningBars small className="text-white" /> : <Music size={18} aria-hidden="true" />}
          {listening ? 'Listening in' : 'Identify this song'}
        </button>
        <p className="mt-2.5 px-2 text-center text-[12px] leading-[17px] text-textMuted [text-wrap:balance]">
          {supported ? 'Listens to the stream for a few seconds' : refusal}
        </p>
      </div>
      {total === 0 ? (
        <p className="px-4 pb-6 pt-5 text-center text-[13.5px] font-medium text-textSecondary">No songs yet</p>
      ) : (
        <>
          {recent.length > 0 && (
            <div className="sn-songs-scroll min-h-0 flex-1 overflow-y-auto px-2 pb-2">
              <h3 className="flex h-9 items-end px-3 pb-1.5 text-[11.5px] font-semibold uppercase leading-4 tracking-[0.06em] text-textMuted">
                Recent
              </h3>
              {recent.map((row) => {
                const tint = tintFor(row);
                return (
                  <button
                    key={row.key}
                    type="button"
                    onClick={() => onOpenSong(row.key)}
                    style={{ '--row-tint': tint ?? 'var(--color-surface-hover)' } as CSSProperties}
                    className="flex w-full items-center gap-3.5 rounded-xl px-2.5 py-2.5 text-left outline-none transition-[background-color,transform] duration-150 focus-visible:bg-[color:var(--row-tint)] [@media(hover:hover)]:hover:bg-[color:var(--row-tint)] [html[data-motion=full]_&]:active:scale-[0.985]"
                  >
                    <RecentCover key={row.art ?? ''} src={row.art} fallback={row.art_full} />
                    <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                      <span className="truncate text-[14px] font-semibold leading-[19px] text-textPrimary">{row.title}</span>
                      <span className="truncate text-[13px] leading-[18px] text-textSecondary">{row.artist}</span>
                    </span>
                    <span className="shrink-0 self-start pt-0.5 text-[12px] tabular-nums text-textMuted">{heardAgo(row.last_ms)}</span>
                  </button>
                );
              })}
            </div>
          )}
          <button
            type="button"
            onClick={onOpenAll}
            className="flex h-[52px] shrink-0 items-center gap-2 border-t border-borderSubtle px-4 text-left outline-none transition-colors duration-150 focus-visible:bg-glass-hover [@media(hover:hover)]:hover:bg-glass-hover"
          >
            <span className="flex-1 text-[13.5px] font-semibold text-textPrimary">All your songs</span>
            <ChevronRight size={17} className="text-textMuted" aria-hidden="true" />
          </button>
        </>
      )}
    </>
  );
}

// The last answer, so a popover opened again shows at once (the player's
// overlay row, and this with it, unmounts while the overlay is hidden).
let lastPage: { rows: SongRow[]; total: number | null } | null = null;
/** How long a first open waits for its songs before it shows without them. */
const FIRST_WAIT_MS = 250;

export interface SongsFlyoutProps {
  /** Where it opens, or null while it is closed. */
  placement: Placement | null;
  /** The arrow that opens it: a press on it is not outside. */
  arrowRef: RefObject<HTMLElement | null>;
  onClose: () => void;
  onIdentify: () => void;
  supported: boolean;
  refusal: string;
}

/** The popover: portaled, fixed, closed by a press outside it, Escape, the
 *  window losing focus or going out of view, a resize, a full-screen toggle,
 *  or anything chosen in it. */
export function SongsFlyout({ placement, arrowRef, onClose, onIdentify, supported, refusal }: SongsFlyoutProps) {
  const open = placement !== null;
  const flyoutRef = useRef<HTMLDivElement>(null);
  const [page, setPage] = useState(lastPage);
  const [waited, setWaited] = useState(false);
  const listening = useSyncExternalStore(subscribeListening, getListening, noListening);
  const version = useSyncExternalStore(subscribeTheme, themeVersion, serverVersion);
  const fullscreen = useAppStore((s) => s.isPlayerFullscreen);
  const pressedInside = useOutsidePress(open, [flyoutRef, arrowRef], onClose);

  // Heard first, then asked, so a change between the two is not missed; every
  // change while open asks again, and only the latest answer counts.
  useEffect(() => {
    if (!open) return;
    let alive = true;
    let ticket = 0;
    const load = () => {
      const mine = ++ticket;
      fetchSongPage('', 0, RECENT)
        .then((answer) => {
          if (!alive || mine !== ticket) return;
          lastPage = { rows: answer.rows.slice(0, RECENT), total: answer.total };
          setPage(lastPage);
        })
        .catch((e: unknown) => {
          Logger.warn('[Songs] recent songs failed:', e);
          if (alive && mine === ticket) setPage((was) => was ?? { rows: [], total: null });
        });
    };
    const unlisten = onSongHistoryChanged(() => load());
    void unlisten.then(() => alive && load()).catch(() => alive && load());
    const wait = setTimeout(() => alive && setWaited(true), FIRST_WAIT_MS);
    return () => {
      alive = false;
      clearTimeout(wait);
      void unlisten.then((stop) => stop()).catch(() => {});
    };
  }, [open]);

  // Escape closes it (focus stays on the arrow, so the window hears it first,
  // and the same press leaves full screen alone); so do the window losing
  // focus or going out of view, and a resize or full-screen toggle, which move
  // what it hangs from.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.preventDefault();
      e.stopPropagation();
      onClose();
    };
    const onHidden = () => {
      if (isWindowHidden()) onClose();
    };
    window.addEventListener('keydown', onKey, true);
    window.addEventListener('blur', onClose);
    window.addEventListener('resize', onClose);
    const stopVisibility = onWindowVisibility(onHidden);
    const stopStore = useAppStore.subscribe((s, was) => {
      if (s.isPlayerFullscreen !== was.isPlayerFullscreen) onClose();
    });
    return () => {
      window.removeEventListener('keydown', onKey, true);
      window.removeEventListener('blur', onClose);
      window.removeEventListener('resize', onClose);
      stopVisibility();
      stopStore();
    };
  }, [open, onClose]);

  const tintFor = useCallback(
    (row: SongRow) => {
      void version;
      return rowTint(row.tint, readSurface('flyout'));
    },
    [version],
  );

  if (typeof document === 'undefined') return null;
  const shown = !!placement && (!!page || waited);
  const choose = (then: () => void) => {
    onClose();
    then();
  };
  return createPortal(
    <AnimatePresence>
      {shown && (
        <motion.div
          key="songs-flyout"
          ref={flyoutRef}
          {...{ [SONGS_FLYOUT_ATTR]: '' }}
          role="dialog"
          aria-label="Your songs"
          onPointerDownCapture={pressedInside}
          // The player's own right-click menu stays shut.
          onContextMenu={(e) => e.stopPropagation()}
          className="glass-flyout no-live-blur fixed flex flex-col overflow-hidden !rounded-2xl !p-0 text-left"
          style={{
            top: placement.top,
            left: placement.left,
            width: placement.width,
            maxHeight: placement.maxHeight,
            zIndex: flyoutZIndex(fullscreen),
            transformOrigin: 'top right',
          }}
          initial={{ opacity: 0, scale: 0.96, y: -6 }}
          animate={{ opacity: 1, scale: 1, y: 0, transition: { duration: 0.18, ease: EASE } }}
          exit={{ opacity: 0, scale: 0.97, y: -4, transition: { duration: 0.12, ease: EASE } }}
        >
          <SongsFlyoutBody
            recent={page?.rows ?? []}
            total={page ? page.total : null}
            supported={supported}
            refusal={refusal}
            listening={listening !== null}
            tintFor={tintFor}
            onIdentify={() => choose(onIdentify)}
            onOpenSong={(key) => choose(() => openSongs(key))}
            onOpenAll={() => choose(() => openSongs())}
          />
        </motion.div>
      )}
    </AnimatePresence>,
    document.body,
  );
}

export interface SongsSplitButtonProps {
  onIdentify: () => void;
  supported: boolean;
  refusal: string;
}

/** The note and the arrow in one glazed control, and the popover it opens. */
export function SongsSplitButton({ onIdentify, supported, refusal }: SongsSplitButtonProps) {
  const listening = useSyncExternalStore(subscribeListening, getListening, noListening);
  const [placement, setPlacement] = useState<Placement | null>(null);
  const splitRef = useRef<HTMLDivElement>(null);
  const arrowRef = useRef<HTMLButtonElement>(null);
  const close = useCallback(() => setPlacement(null), []);
  // Placed as it opens; anything that would move it (a resize, a full-screen
  // toggle) closes it instead.
  const toggle = () => {
    const rect = splitRef.current?.getBoundingClientRect();
    setPlacement(placement || !rect ? null : placeFlyout(rect, { w: window.innerWidth, h: window.innerHeight }));
  };

  const capturing = listening?.phase === 'listening';
  const finding = listening?.phase === 'finding';
  const noteLabel = !supported
    ? refusal
    : capturing
      ? 'Stop listening'
      : finding
        ? 'Looking it up'
        : 'Identify song';
  const open = placement !== null;
  // The note, the bars and the spinner cross-fade in one place.
  const noteKey = capturing ? `bars-${listening.id}` : finding ? 'finding' : 'note';

  return (
    <div ref={splitRef} className="flex items-stretch chrome-glaze chrome-glaze--control chrome-glaze--frosted">
      <Tooltip content={noteLabel} side="bottom">
        <button
          type="button"
          aria-label={supported ? noteLabel : 'Identify song'}
          onClick={capturing ? cancelListening : onIdentify}
          disabled={!supported || finding}
          className={`group flex aspect-square items-center justify-center p-2 ${finding ? 'cursor-wait' : ''} ${!supported ? 'cursor-default opacity-40' : ''}`}
        >
          <span className="relative block h-4 w-4">
            <AnimatePresence initial={false}>
              <motion.span
                key={noteKey}
                className="absolute inset-0 flex items-center justify-center"
                initial={{ opacity: 0, scale: 0.8 }}
                animate={{ opacity: 1, scale: 1 }}
                exit={{ opacity: 0, scale: 0.8 }}
                transition={{ duration: 0.15, ease: EASE }}
              >
                {capturing ? (
                  <ListeningBars small className="text-white" />
                ) : finding ? (
                  <Loader2 className="h-4 w-4 animate-spin text-accent" />
                ) : (
                  <Music className="h-4 w-4 text-white transition-colors duration-200 group-hover:text-accent" />
                )}
              </motion.span>
            </AnimatePresence>
          </span>
        </button>
      </Tooltip>
      <Tooltip content="Your songs" side="bottom" disabled={open}>
        <button
          ref={arrowRef}
          type="button"
          aria-label="Your songs"
          aria-haspopup="dialog"
          aria-expanded={open}
          onClick={toggle}
          className="flex w-5 items-center justify-center border-l border-white/[0.14] pr-0.5 text-white/70 transition-colors duration-200 hover:text-white"
        >
          <ChevronDown
            size={14}
            strokeWidth={2.4}
            className={`transition-transform duration-150 ${open ? 'rotate-180' : ''}`}
            aria-hidden="true"
          />
        </button>
      </Tooltip>
      <SongsFlyout
        placement={placement}
        arrowRef={arrowRef}
        onClose={close}
        onIdentify={onIdentify}
        supported={supported}
        refusal={refusal}
      />
    </div>
  );
}
