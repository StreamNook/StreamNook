// The listening pill over a chat's last messages while a song is identified
// for that chat: a glass shape floating at the foot of the messages panel (it
// takes no room, and never covers the moderator strips or the composer below
// it), with bars that move with the stream's sound, a short phrase
// ("Ears up", picked per capture, listeningWords.ts), and a quiet Cancel ringed
// by how far the listening has come; then another phrase ("Let me cook") while
// the clip is looked up; then the answer (SongResultCard) in the same shape,
// which goes after a few seconds. One
// object whose contents change. The song's card still lands in the chat as its
// record. Desktop only: the phones keep their chat line.
//
// The listening state comes through useSyncExternalStore; the sound level
// never does. listeningBars.ts draws every set of bars from one frame loop,
// writing their transforms straight to the elements, so the pill renders once
// per phase, not per level.
import './listeningStrip.css';
import { memo, useCallback, useEffect, useLayoutEffect, useRef, useSyncExternalStore } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { X } from 'lucide-react';
import { getListening, subscribeListening, type Listening } from '../../utils/songListening';
import { dismissSongResult, songResultFor, subscribeSongResults, type SongResult } from '../../utils/songResult';
import { IS_MOBILE } from '../../utils/platform';
import { BAR_WEIGHTS, drawBars, motionMode } from './listeningBars';
import { isOngoing, phraseFor } from './listeningWords';
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

/** The ring's radius and length, in a 30 px box. */
const RING_R = 14;
const RING_C = 2 * Math.PI * RING_R;

/**
 * Cancel, with how far this attempt's listening has come as a ring around it:
 * from 12 o'clock, clockwise, one transition per attempt (a 30 px SVG, so its
 * repaint is nothing). Under Reduced Motion the ring moves in quarters; under
 * Off there is no ring, only the X. While the clip is looked up neither shows
 * (the bars' slow wave says it is working), and its place is kept.
 */
function CancelRing({ listening, onCancel }: { listening: Listening; onCancel: (() => void) | null }) {
  const arcRef = useRef<SVGCircleElement>(null);
  const { phase, startedAt, captureMs, attempt } = listening;
  const mode = motionMode();
  useLayoutEffect(() => {
    const arc = arcRef.current;
    if (!arc) return;
    arc.style.transition = 'none';
    if (startedAt === null || !(captureMs > 0)) {
      arc.style.strokeDashoffset = String(RING_C);
      return;
    }
    const done = Math.min(1, Math.max(0, (Date.now() - startedAt) / captureMs));
    const quarters = mode === 'reduced';
    const from = quarters ? Math.floor(done * 4) / 4 : done;
    arc.style.strokeDashoffset = String(RING_C * (1 - from));
    // Drawn where it is before it moves, so the transition starts there.
    void arc.getBoundingClientRect();
    const left = Math.round(captureMs * (1 - done));
    const steps = Math.max(1, 4 - Math.floor(done * 4));
    arc.style.transition = `stroke-dashoffset ${left}ms ${quarters ? `steps(${steps}, end)` : 'linear'}`;
    arc.style.strokeDashoffset = '0';
  }, [startedAt, captureMs, attempt, mode]);
  const finding = phase === 'finding';
  return (
    <button
      type="button"
      onClick={onCancel ?? undefined}
      disabled={!onCancel}
      tabIndex={onCancel ? undefined : -1}
      aria-hidden={onCancel ? undefined : true}
      aria-label="Cancel"
      className={`relative flex h-[30px] w-[30px] shrink-0 items-center justify-center rounded-full text-textMuted transition-[background-color,color,opacity] duration-150 hover:bg-glass-hover hover:text-textPrimary ${onCancel && !finding ? '' : 'pointer-events-none opacity-0'}`}
    >
      {mode !== 'off' && (
        <svg aria-hidden="true" width={30} height={30} viewBox="0 0 30 30" className="pointer-events-none absolute inset-0 -rotate-90">
          <circle cx={15} cy={15} r={RING_R} fill="none" stroke="currentColor" strokeOpacity={0.16} strokeWidth={2} />
          <circle
            ref={arcRef}
            cx={15}
            cy={15}
            r={RING_R}
            fill="none"
            stroke="var(--color-accent)"
            strokeWidth={2}
            strokeDasharray={RING_C}
            strokeDashoffset={RING_C}
          />
        </svg>
      )}
      <X size={14} strokeWidth={2.2} />
    </button>
  );
}

/** What the shape holds for a listening that answers into this chat. */
export function ListeningStripView({ listening, onCancel }: { listening: Listening; onCancel: (() => void) | null }) {
  // Its words, picked for this capture: one for listening, one for finding.
  const words = phraseFor(listening.phase === 'finding' ? 'finding' : 'listening', listening.id);
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
            {/* Dots only after words that are still going. */}
            {isOngoing(words) && (
              <span className="sn-listen-dots" aria-hidden="true">
                <i>.</i>
                <i>.</i>
                <i>.</i>
              </span>
            )}
          </motion.span>
        </AnimatePresence>
      </span>
      {/* Held in place while the song is looked up, so nothing shifts. */}
      <CancelRing listening={listening} onCancel={onCancel} />
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

/** Mounted at the foot of every chat's messages with that chat's room (its
 *  chat-slice key); shows while a listening answers into that room, and its
 *  answer for a few seconds after. Its room is a stable string, so the chat's
 *  own renders pass it by. */
export const ListeningStrip = memo(function ListeningStrip({ room }: { room: string | null }) {
  const listening = useSyncExternalStore(subscribeListening, getListening, noListening);
  // An answer older than its card's time is old news (it was filed while this
  // chat was not on screen), unless its card is held under the pointer.
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
    // Anchored inside the messages panel, 8 px above its foot: the shape
    // floats over the last messages and never over the strips or the
    // composer below, and nothing moves when it comes or goes. Only the shape
    // itself takes the pointer. While it shows, Chat Paused rises above it
    // (listeningStrip.css, by the shape's mark).
    <div className="pointer-events-none absolute inset-x-2 bottom-2 z-30 flex justify-center">
      <AnimatePresence custom={off}>
        {contents && (
          <motion.div
            key="shape"
            data-listening-shape=""
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
});

export default ListeningStrip;
