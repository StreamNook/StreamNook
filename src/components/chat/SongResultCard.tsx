// The answer over a chat's composer, in the listening pill's own floating
// shape (ListeningStrip draws the shape; this draws what is inside it), so the
// pill turning into the answer reads as one object changing. A song found: its
// cover on a soft wash of its own colors, a short label ("Got it") in the
// cover's ink, its name on one line, up to three of its services, and
// dismiss; the row itself opens the song in Your songs. A miss or an error:
// the same row with an icon, its words, and Try again for a miss (the words
// are picked per answer, listeningWords.ts). It shows for a few seconds, held while
// the pointer or the keyboard is on it, then goes (ListeningStrip runs the
// leave). The song's card in the chat stays as the record.
import { useEffect, useRef, useState, useSyncExternalStore, type KeyboardEvent, type MouseEvent, type ReactNode } from 'react';
import { motion } from 'framer-motion';
import { CircleAlert, Music, X } from 'lucide-react';
import { CARD_LAYERS, useCoverFit, washBackground, type CoverTint } from '../../utils/coverTint';
import { fetchSongPage, hasSongsSurface, onSongHistoryChanged, openSongs, subscribeSongsSurface } from '../../utils/songHistory';
import { openSongLink } from '../../utils/songLinks';
import { holdSongResult, type SongResult } from '../../utils/songResult';
import { SocialLogo } from '../ui/SocialLogo';
import { Tooltip } from '../ui/Tooltip';
import { GLAZE_PILL } from '../ui/glazeButtons';
import { markedLinks } from '../songs/songLinks';
import { markOf, markSize } from '../songs/serviceMarks';
import { phraseFor } from './listeningWords';
import { ANSWER_ROW, identifyAgain, RESULT_MS } from './stripRoom';

const noSurface = () => false;

/** The wash keeps to the row's left, behind the cover and the words. */
const WASH_MASK = 'linear-gradient(to right, #000 0%, #000 30%, transparent 78%)';

/** Shows for `ms`, held while hovered or focused, then `onDone`. The store
 *  hears the hold, so answer `id` outlives its time while held. */
function useShownFor(ms: number, onDone: () => void, id: number) {
  const [held, setHeld] = useState(false);
  const left = useRef(ms);
  useEffect(() => {
    if (held) return;
    const started = Date.now();
    const t = setTimeout(onDone, Math.max(0, left.current));
    return () => {
      clearTimeout(t);
      left.current -= Date.now() - started;
    };
  }, [held, onDone]);
  useEffect(() => {
    holdSongResult(id, held);
    return () => holdSongResult(id, false);
  }, [id, held]);
  return {
    onPointerEnter: () => setHeld(true),
    onPointerLeave: () => setHeld(false),
    onFocusCapture: () => setHeld(true),
    onBlurCapture: () => setHeld(false),
  };
}

/** The found song's stored colors, once Rust has read its cover: they land a
 *  moment after the song is filed (a `tinted` change names it). */
function useStoredTint(key: string | null | undefined): CoverTint | null | undefined {
  const [tint, setTint] = useState<{ key: string; tint: CoverTint | null } | null>(null);
  useEffect(() => {
    if (!key) return;
    let alive = true;
    // The song was just filed, so it heads the list.
    const read = () =>
      fetchSongPage('', 0, 3)
        .then((page) => {
          const row = page.rows.find((r) => r.key === key);
          if (alive && row?.tint !== undefined) setTint({ key, tint: row.tint });
        })
        .catch(() => {});
    void read();
    const unlisten = onSongHistoryChanged((change) => {
      if (change.kind === 'tinted' && change.keys.includes(key)) void read();
    });
    return () => {
      alive = false;
      void unlisten.then((stop) => stop()).catch(() => {});
    };
  }, [key]);
  return tint && tint.key === key ? tint.tint : undefined;
}

/** A press on one of the row's own buttons is that button's, never the
 *  row's (which would open the song). */
const own = (act: () => void) => (e: MouseEvent) => {
  e.stopPropagation();
  act();
};

/** The quiet round dismiss at the row's end. */
function Dismiss({ onClick }: { onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={own(onClick)}
      aria-label="Dismiss"
      // The listening pill's Cancel box, so the X stays put as the contents change.
      className="flex h-[30px] w-[30px] shrink-0 items-center justify-center rounded-full text-textMuted transition-colors duration-150 hover:bg-glass-hover hover:text-textPrimary"
    >
      <X size={14} strokeWidth={2.2} />
    </button>
  );
}

/** The 40 px slot on the row's left, for an icon where a cover would be. */
function IconSlot({ children }: { children: ReactNode }) {
  return (
    <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-[10px] bg-glass-hover text-textMuted" aria-hidden="true">
      {children}
    </span>
  );
}

/** A music note struck through: nothing was heard. */
function NoSong() {
  return (
    <span className="relative flex h-[18px] w-[18px] items-center justify-center">
      <Music size={17} strokeWidth={2} />
      <span className="absolute left-[-1px] top-1/2 h-0.5 w-[21px] -translate-y-1/2 rotate-45 rounded-full bg-current" />
    </span>
  );
}

/** A song found. */
function FoundRow({ answer, onDone }: { answer: SongResult & { result: { status: 'match' } }; onDone: () => void }) {
  const song = answer.result.song;
  const hold = useShownFor(RESULT_MS.match, onDone, answer.id);
  const [host, setHost] = useState<HTMLDivElement | null>(null);
  const [box, setBox] = useState({ w: 0, h: 0 });
  useEffect(() => {
    if (!host) return;
    const ro = new ResizeObserver(([entry]) => {
      const w = Math.round(entry.contentRect.width);
      const h = Math.round(entry.contentRect.height);
      setBox((was) => (was.w === w && was.h === h ? was : { w, h }));
    });
    ro.observe(host);
    return () => ro.disconnect();
  }, [host]);
  const tint = useStoredTint(song.key);
  const fit = useCoverFit(tint, 'flyout', box.w, box.h, CARD_LAYERS, true);
  const canOpen = useSyncExternalStore(subscribeSongsSurface, hasSongsSurface, noSurface) && !!song.key;
  // A narrow chat keeps room for the song's name: two services there.
  const services = markedLinks(song.providers, box.w > 0 && box.w < 360 ? 2 : 3);
  const ink = fit?.ink ?? 'var(--color-accent-on-surface)';
  const open = () => {
    if (!canOpen) return;
    openSongs(song.key ?? null);
    onDone();
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.target !== e.currentTarget || (e.key !== 'Enter' && e.key !== ' ')) return;
    e.preventDefault();
    open();
  };
  const face = (
    <span className="flex min-w-0 flex-1 items-center gap-3">
      <span className="relative h-10 w-10 shrink-0 rounded-[10px] shadow-[0_6px_14px_-6px_rgba(0,0,0,0.6)]">
        {song.album_art ? (
          <img src={song.album_art} alt="" width={40} height={40} className="h-10 w-10 rounded-[10px] object-cover" />
        ) : (
          <IconSlot>
            <Music size={17} strokeWidth={2} />
          </IconSlot>
        )}
      </span>
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="text-[11px] font-semibold uppercase leading-[14px] tracking-[0.06em]" style={{ color: ink }}>
          {phraseFor('found', answer.id)}
        </span>
        <span className="truncate leading-[18px]">
          <span className="text-[13.5px] font-semibold text-textPrimary">{song.title}</span>
          <span className="text-[13px] text-textSecondary"> · {song.artist}</span>
        </span>
      </span>
    </span>
  );
  return (
    <div
      {...hold}
      data-song-open={canOpen ? '' : undefined}
      role={canOpen ? 'button' : 'status'}
      tabIndex={canOpen ? 0 : undefined}
      aria-label={canOpen ? `${song.title} by ${song.artist}. Open in Your songs` : undefined}
      onClick={open}
      onKeyDown={onKey}
      className={`${ANSWER_ROW} group outline-none focus-visible:outline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-[color:var(--color-accent)] ${canOpen ? 'cursor-pointer' : ''}`}
    >
      <div ref={setHost} aria-hidden="true" className="pointer-events-none absolute inset-0" style={{ maskImage: WASH_MASK, WebkitMaskImage: WASH_MASK }}>
        {fit && (
          <motion.div
            className="absolute inset-0"
            style={{ background: washBackground(fit, box, { x: 0, y: 0, w: box.w, h: box.h }) }}
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            transition={{ duration: 0.3, ease: 'easeOut' }}
          />
        )}
      </div>
      {/* Its base brightens a touch under the pointer. */}
      {canOpen && (
        <div
          aria-hidden="true"
          className="pointer-events-none absolute inset-0 bg-white/[0.035] opacity-0 transition-opacity duration-150 [@media(hover:hover)]:group-hover:opacity-100"
        />
      )}
      <span className="relative flex min-w-0 flex-1 items-center gap-3">
        {canOpen ? (
          <Tooltip content="Open in Your songs" side="top">
            {face}
          </Tooltip>
        ) : (
          face
        )}
        <span className="flex shrink-0 items-center gap-1.5">
          {services.map((l) => (
            <Tooltip key={l.url} content={l.name} side="top">
              <button
                type="button"
                aria-label={`Open in ${l.name}`}
                onClick={own(() => void openSongLink(l.url))}
                className={`${GLAZE_PILL} flex h-[30px] w-[30px] items-center justify-center`}
              >
                <SocialLogo platform={markOf(l.name)} size={markSize(l.name, 17)} />
              </button>
            </Tooltip>
          ))}
          <Dismiss onClick={onDone} />
        </span>
      </span>
    </div>
  );
}

/** Nothing found, or why it could not try. */
function OtherRow({ answer, onDone }: { answer: SongResult; onDone: () => void }) {
  const hold = useShownFor(RESULT_MS.other, onDone, answer.id);
  const miss = answer.result.status === 'no-match';
  const text = answer.result.status === 'error' ? answer.result.message : phraseFor('miss', answer.id);
  return (
    <div {...hold} role="status" className={ANSWER_ROW}>
      <IconSlot>{miss ? <NoSong /> : <CircleAlert size={18} strokeWidth={2} />}</IconSlot>
      <span className="min-w-0 flex-1 truncate text-[13.5px] font-semibold leading-[18px] text-textPrimary">{text}</span>
      <span className="flex shrink-0 items-center gap-1.5">
        {miss && (
          <button
            type="button"
            onClick={own(() => void identifyAgain(answer.room))}
            className={`${GLAZE_PILL} flex h-[30px] items-center px-2.5 text-[12.5px]`}
          >
            Try again
          </button>
        )}
        <Dismiss onClick={onDone} />
      </span>
    </div>
  );
}

/** What the shape holds for an answer. */
export function SongResultView({ answer, onDone }: { answer: SongResult; onDone: () => void }) {
  if (answer.result.status === 'match') {
    return <FoundRow answer={answer as SongResult & { result: { status: 'match' } }} onDone={onDone} />;
  }
  return <OtherRow answer={answer} onDone={onDone} />;
}
