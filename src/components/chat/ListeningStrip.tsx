// The listening pill in a chat's composer shelf while a song is identified
// for that chat: a floating glass pill with bars that move with the stream's
// sound, "Listening for the song", how far the listening has come along its
// foot, and a quiet Cancel; then "Finding the song" while the clip is looked
// up. When the answer comes back the pill gives its place to the answer's card
// (SongResultCard), which folds down into the composer after a few seconds.
// The song's card still lands in the chat as its record. Desktop only: the
// phones keep their chat line.
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
import { SongResultView } from './SongResultCard';
import { ANSWER_GLOW, stripCancel, stripShows } from './stripRoom';

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

/** The pill itself, for a listening that answers into this chat. */
export function ListeningStripView({ listening, onCancel }: { listening: Listening; onCancel: (() => void) | null }) {
  const words = pillWords(listening);
  return (
    <div
      role="status"
      className="glass-flyout no-live-blur relative mb-2 flex h-11 items-center gap-3 overflow-hidden !rounded-full !py-0 !pl-4 !pr-1.5"
      style={ANSWER_GLOW}
    >
      <ListeningBars key={listening.id} className="sn-listen-bars--accent" />
      {/* Listening and finding cross-fade in one place. */}
      <span className="grid min-w-0 flex-1">
        <AnimatePresence initial={false}>
          <motion.span
            key={words}
            className="col-start-1 row-start-1 truncate text-[13px] font-semibold leading-[18px] text-textPrimary"
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
        className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-full text-textMuted transition-[background-color,color,opacity] duration-150 hover:bg-glass-hover hover:text-textPrimary ${onCancel ? '' : 'pointer-events-none opacity-0'}`}
      >
        <X size={15} strokeWidth={2.2} />
      </button>
      <TimeLine listening={listening} />
    </div>
  );
}

const noListening = (): Listening | null => null;
const noAnswer = (): SongResult | null => null;

/**
 * How the strip's pieces come and go. The pill rises in and fades out; an
 * answer's card folds down into the composer when its time is up (under the
 * Reduced Motion setting it only fades, and under Off it goes at once), and
 * goes at once when a new listening takes its place.
 */
type How = { replaced: boolean; off: boolean };
const PILL = {
  enter: { opacity: 0, y: 8 },
  shown: { opacity: 1, y: 0, transition: { duration: 0.16, ease: EASE } },
  leave: ({ off }: How) => ({ opacity: 0, y: 4, transition: { duration: off ? 0 : 0.12, ease: EASE } }),
};
const ANSWER = {
  enter: { opacity: 0, y: 8 },
  shown: { opacity: 1, y: 0, scaleY: 1, transition: { duration: 0.16, ease: EASE } },
  leave: ({ replaced, off }: How) =>
    off || replaced
      ? { opacity: 0, transition: { duration: off ? 0 : 0.08 } }
      : { opacity: 0, scaleY: 0.6, y: 10, transition: { duration: 0.26, ease: EASE } },
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
  if (IS_MOBILE) return null;
  const how: How = { replaced: !!live, off: motionMode() === 'off' };
  return (
    // The leaving piece steps out of the layout, so the next one takes its
    // place at once: the pill turns into the answer where it stood.
    <AnimatePresence mode="popLayout" custom={how}>
      {live ? (
        <motion.div
          key={`listening-${live.id}`}
          custom={how}
          variants={PILL}
          initial="enter"
          animate="shown"
          exit="leave"
        >
          <ListeningStripView listening={live} onCancel={stripCancel(live)} />
        </motion.div>
      ) : answer ? (
        <motion.div
          key={`answer-${answer.id}`}
          custom={how}
          variants={ANSWER}
          initial="enter"
          animate="shown"
          exit="leave"
          style={{ transformOrigin: 'bottom center' }}
        >
          <SongResultView answer={answer} onDone={done} />
        </motion.div>
      ) : null}
    </AnimatePresence>
  );
}

export default ListeningStrip;
