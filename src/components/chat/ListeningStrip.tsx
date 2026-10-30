// The listening pill over a chat's composer while a song is identified for
// that chat: a glass shape floating just above the composer (it takes no room
// in the shelf), with bars that move with the stream's sound, "Listening for
// the song", how far the listening has come along its foot, and a quiet
// Cancel; then "Finding the song" while the clip is looked up; then the answer
// (SongResultCard) in the same shape, which goes after a few seconds. One
// object whose contents change. The song's card still lands in the chat as its
// record. Desktop only: the phones keep their chat line.
//
// The listening state comes through useSyncExternalStore; the sound level
// never does. listeningBars.ts draws every set of bars from one frame loop,
// writing their transforms straight to the elements, so the pill renders once
// per phase, not per level.
import './listeningStrip.css';
import { useCallback, useEffect, useLayoutEffect, useRef, useSyncExternalStore } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { X } from 'lucide-react';
import { getListening, subscribeListening, type Listening } from '../../utils/songListening';
import { dismissSongResult, songResultFor, subscribeSongResults, type SongResult } from '../../utils/songResult';
import { IS_MOBILE } from '../../utils/platform';
import { BAR_WEIGHTS, drawBars, motionMode } from './listeningBars';
import { subscribeTheme } from '../../themes';
import { readSurface } from '../../utils/coverTint';
import { SongResultView } from './SongResultCard';
import { ANSWER_ROW, ANSWER_SHELL, answerShadow, stripCancel, stripShows } from './stripRoom';

/** The house easing for these moves. */
const EASE = [0.2, 0.8, 0.2, 1] as const;

/**
 * Five bars, drawn by the shared renderer while a listening exists: they
 * breathe until the sound arrives, then follow it; they run a slow wave while
 * the song is looked up. `small` fits them in the player's 16 px icon box.
 */
export function ListeningBars({ small = false, className = '' }: { small?: boolean; className?: string }) {
  const rootRef = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    return drawBars(Array.from(root.children) as HTMLElement[]);
  }, []);
  return (
    <span
      ref={rootRef}
      className={['sn-listen-bars', small ? 'sn-listen-bars--small' : '', className].filter(Boolean).join(' ')}
      aria-hidden="true"
    >
      {BAR_WEIGHTS.map((_, i) => (
        <i key={i} />
      ))}
    </span>
  );
}

/** How far this attempt's listening has come: one transform transition from
 *  where it is now to the end, started again for each attempt. */
function TimeLine({ listening }: { listening: Listening }) {
  const lineRef = useRef<HTMLDivElement>(null);
  const { phase, startedAt, captureMs, attempt } = listening;
  useLayoutEffect(() => {
    const line = lineRef.current;
    if (!line) return;
    // With Motion off a filling line would only jump to its end.
    line.style.opacity = motionMode() === 'off' ? '0' : '';
    if (phase === 'finding') {
      line.style.transition = 'transform 200ms ease-out';
      line.style.transform = 'scaleX(1)';
      return;
    }
    line.style.transition = 'none';
    if (startedAt === null || !(captureMs > 0)) {
      line.style.transform = 'scaleX(0)';
      return;
    }
    const done = Math.min(1, Math.max(0, (Date.now() - startedAt) / captureMs));
    line.style.transform = `scaleX(${done})`;
    // Drawn where it is before it moves, so the transition starts there.
    void line.offsetWidth;
    line.style.transition = `transform ${Math.round(captureMs * (1 - done))}ms linear`;
    line.style.transform = 'scaleX(1)';
  }, [phase, startedAt, captureMs, attempt]);
  return <div ref={lineRef} aria-hidden="true" className="sn-listen-time" />;
}

/** What the pill says for a listening. */
function pillWords(listening: Listening): string {
  if (listening.phase === 'finding') return 'Finding the song';
  return listening.attempt > 1 ? 'Trying again' : 'Listening for the song';
}

/** What the shape holds for a listening that answers into this chat. */
export function ListeningStripView({ listening, onCancel }: { listening: Listening; onCancel: (() => void) | null }) {
  const words = pillWords(listening);
  return (
    <div role="status" className={ANSWER_ROW}>
      {/* The bars sit in the slot the answer's cover takes. */}
      <span className="flex h-10 w-10 shrink-0 items-center justify-center" aria-hidden="true">
        <ListeningBars key={listening.id} className="sn-listen-bars--accent" />
      </span>
      {/* Listening and finding cross-fade in one place. */}
      <span className="grid min-w-0 flex-1">
        <AnimatePresence initial={false}>
          <motion.span
            key={words}
            className="col-start-1 row-start-1 truncate text-[13.5px] font-semibold leading-[18px] text-textPrimary"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.16, ease: EASE }}
          >
            {words}
            <span className="sn-listen-dots" aria-hidden="true">
              <i>.</i>
              <i>.</i>
              <i>.</i>
            </span>
          </motion.span>
        </AnimatePresence>
      </span>
      {/* Held in place while the song is looked up, so nothing shifts. */}
      <button
        type="button"
        onClick={onCancel ?? undefined}
        disabled={!onCancel}
        tabIndex={onCancel ? undefined : -1}
        aria-hidden={onCancel ? undefined : true}
        aria-label="Cancel"
        className={`flex h-7 w-7 shrink-0 items-center justify-center rounded-full text-textMuted transition-[background-color,color,opacity] duration-150 hover:bg-glass-hover hover:text-textPrimary ${onCancel ? '' : 'pointer-events-none opacity-0'}`}
      >
        <X size={15} strokeWidth={2.2} />
      </button>
      <TimeLine listening={listening} />
    </div>
  );
}

const noListening = (): Listening | null => null;
const noAnswer = (): SongResult | null => null;
const lightTheme = () => readSurface('flyout').light;
const darkOnServer = () => false;

/**
 * How the floating shape comes and goes: it rises in with a fade, and leaves
 * (an answer's time up, dismissed, a Cancel) with a fade, a small drop and a
 * slight shrink toward the composer. The Reduced Motion setting keeps only the
 * fades (MotionScope); under Off it comes and goes at once.
 */
const SHAPE = {
  enter: { opacity: 0, y: 8, scale: 1 },
  shown: (off: boolean) => ({ opacity: 1, y: 0, scale: 1, transition: { duration: off ? 0 : 0.16, ease: EASE } }),
  leave: (off: boolean) => ({ opacity: 0, y: 6, scale: 0.98, transition: { duration: off ? 0 : 0.2, ease: EASE } }),
};

/** Mounted in every chat's composer shelf with that chat's room (its
 *  chat-slice key); shows while a listening answers into that room, and its
 *  answer for a few seconds after. */
export function ListeningStrip({ room }: { room: string | null }) {
  const listening = useSyncExternalStore(subscribeListening, getListening, noListening);
  const stored = useSyncExternalStore(subscribeSongResults, () => songResultFor(room), noAnswer);
  const live = listening && stripShows(listening, room) ? listening : null;
  const answer = live ? null : stored;
  const answerId = answer?.id;
  const done = useCallback(() => {
    if (answerId !== undefined) dismissSongResult(answerId);
  }, [answerId]);
  const light = useSyncExternalStore(subscribeTheme, lightTheme, darkOnServer);
  if (IS_MOBILE) return null;
  const off = motionMode() === 'off';
  const contents = live ? `listening-${live.id}` : answer ? `answer-${answer.id}` : null;
  return (
    // A zero-height anchor: the shape floats over the foot of the chat, 8 px
    // above the composer, so nothing in the shelf or the chat moves when it
    // comes or goes. Only the shape itself takes the pointer.
    <div className="pointer-events-none absolute inset-x-2 bottom-full z-30 mb-4 flex justify-center">
      <AnimatePresence custom={off}>
        {contents && (
          <motion.div
            key="shape"
            custom={off}
            variants={SHAPE}
            initial="enter"
            animate="shown"
            exit="leave"
            className="pointer-events-auto w-full"
            style={{ transformOrigin: 'bottom center' }}
          >
            {/* One shape for listening, finding and the answer: only what is
                inside it cross-fades. */}
            <div
              className={`${ANSWER_SHELL} transition-transform duration-200 ease-[cubic-bezier(0.2,0.8,0.2,1)] [html[data-motion=full]_&:has([data-song-open]:hover)]:-translate-y-px`}
              style={answerShadow(light)}
            >
              <AnimatePresence initial={false}>
                <motion.div
                  key={contents}
                  className="absolute inset-0"
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                  transition={{ duration: off ? 0 : 0.16, ease: EASE }}
                >
                  {live ? (
                    <ListeningStripView listening={live} onCancel={stripCancel(live)} />
                  ) : answer ? (
                    <SongResultView answer={answer} onDone={done} />
                  ) : null}
                </motion.div>
              </AnimatePresence>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

export default ListeningStrip;
