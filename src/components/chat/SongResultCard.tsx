// The answer over a chat's composer, where the listening pill was: a song
// found (its cover on a soft wash of its own colors, "Found it" in the
// cover's ink, its name, up to three of its own links, Open in Your songs),
// or a short pill for a miss or an error. It shows for a few seconds, held
// while the pointer or the keyboard is on it, then folds down into the
// composer (ListeningStrip runs the fold). The song's card in the chat stays
// as the record; this is the moment to catch it.
import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { motion } from 'framer-motion';
import { Music, RotateCcw, X } from 'lucide-react';
import { CARD_LAYERS, useCoverFit, washBackground, type CoverTint } from '../../utils/coverTint';
import { openExternal } from '../../utils/openExternal';
import { fetchSongPage, hasSongsSurface, onSongHistoryChanged, openSongs, subscribeSongsSurface } from '../../utils/songHistory';
import type { SongResult } from '../../utils/songResult';
import { SocialLogo } from '../ui/SocialLogo';
import { Tooltip } from '../ui/Tooltip';
import { markOf, markSize } from '../songs/serviceMarks';
import { ANSWER_GLOW, identifyAgain, RESULT_MS } from './stripRoom';

const noSurface = () => false;

/** Shows for `ms`, held while hovered or focused, then `onDone`. */
function useShownFor(ms: number, onDone: () => void) {
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

/** A quiet round button: dismiss, or a link's logo. */
const ROUND =
  'flex h-8 w-8 shrink-0 items-center justify-center rounded-full text-textMuted transition-colors duration-150 hover:bg-glass-hover hover:text-textPrimary';

function Dismiss({ onClick }: { onClick: () => void }) {
  return (
    <button type="button" onClick={onClick} aria-label="Dismiss" className={ROUND}>
      <X size={15} strokeWidth={2.2} />
    </button>
  );
}

/** A song found: the card. */
function FoundCard({ answer, onDone }: { answer: SongResult & { result: { status: 'match' } }; onDone: () => void }) {
  const song = answer.result.song;
  const hold = useShownFor(RESULT_MS.match, onDone);
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
  const canOpen = useSyncExternalStore(subscribeSongsSurface, hasSongsSurface, noSurface);
  const links = song.providers.filter((p) => !p.search).slice(0, 3);
  const ink = fit?.ink ?? 'var(--color-accent-on-surface)';
  return (
    <div
      {...hold}
      role="status"
      className="glass-flyout no-live-blur relative mb-2 overflow-hidden !rounded-2xl !p-0"
      style={ANSWER_GLOW}
    >
      <div ref={setHost} aria-hidden="true" className="pointer-events-none absolute inset-0">
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
      <div className="relative flex items-start gap-3 py-2.5 pl-2.5 pr-1.5">
        {song.album_art ? (
          <img src={song.album_art} alt="" width={40} height={40} className="mt-0.5 h-10 w-10 shrink-0 rounded-lg object-cover" />
        ) : (
          <span className="mt-0.5 flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-glass-hover" aria-hidden="true">
            <Music size={16} className="text-textMuted" />
          </span>
        )}
        <div className="flex min-w-0 flex-1 flex-col">
          <span className="text-[11px] font-semibold uppercase leading-[15px] tracking-[0.06em]" style={{ color: ink }}>
            Found it
          </span>
          <span className="mt-0.5 truncate text-[13px] leading-[18px]">
            <span className="font-semibold text-textPrimary">{song.title}</span>
            <span className="text-textSecondary"> · {song.artist}</span>
          </span>
          <div className="-ml-1.5 mt-1.5 flex items-center gap-1">
            {links.map((l) => (
              <Tooltip key={l.url} content={l.name} side="top" describe>
                <button
                  type="button"
                  aria-label={`Open in ${l.name}`}
                  onClick={() => void openExternal(l.url)}
                  className={ROUND}
                >
                  <SocialLogo platform={markOf(l.name)} size={markSize(l.name, 16)} />
                </button>
              </Tooltip>
            ))}
            {song.key && canOpen && (
              <button
                type="button"
                onClick={() => {
                  openSongs(song.key ?? null);
                  onDone();
                }}
                className="ml-auto flex h-8 items-center rounded-full px-3 text-[12.5px] font-semibold text-textPrimary transition-colors duration-150 hover:bg-glass-hover"
              >
                Open
              </button>
            )}
          </div>
        </div>
        <Dismiss onClick={onDone} />
      </div>
    </div>
  );
}

/** Nothing found, or why it could not try: the short pill. */
function AnswerPill({ answer, onDone }: { answer: SongResult; onDone: () => void }) {
  const hold = useShownFor(RESULT_MS.other, onDone);
  const miss = answer.result.status === 'no-match';
  const text = answer.result.status === 'error' ? answer.result.message : "Couldn't hear a song";
  return (
    <div
      {...hold}
      role="status"
      className="glass-flyout no-live-blur relative mb-2 flex h-11 items-center gap-2.5 overflow-hidden !rounded-full !py-0 !pl-4 !pr-1.5"
      style={ANSWER_GLOW}
    >
      <Music size={16} className="shrink-0 text-textMuted" aria-hidden="true" />
      <span className="min-w-0 flex-1 truncate text-[13px] font-semibold text-textPrimary">{text}</span>
      {miss && (
        <button
          type="button"
          onClick={() => void identifyAgain(answer.room)}
          className="flex h-8 shrink-0 items-center gap-1.5 rounded-full px-3 text-[12.5px] font-semibold text-textPrimary transition-colors duration-150 hover:bg-glass-hover"
        >
          <RotateCcw size={13} strokeWidth={2.3} aria-hidden="true" />
          Try again
        </button>
      )}
      <Dismiss onClick={onDone} />
    </div>
  );
}

/** The answer's card or pill. */
export function SongResultView({ answer, onDone }: { answer: SongResult; onDone: () => void }) {
  if (answer.result.status === 'match') {
    return <FoundCard answer={answer as SongResult & { result: { status: 'match' } }} onDone={onDone} />;
  }
  return <AnswerPill answer={answer} onDone={onDone} />;
}
