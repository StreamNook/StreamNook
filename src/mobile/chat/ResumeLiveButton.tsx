// What floats over a paused phone chat: the round resume button at the bottom
// right, and the swipe-to-reply hint until it has been learned.
//
// The button is a chevron in a glazed circle; once new messages arrive, the
// chevron turns into how many ("+3", then "99+", so it always fits the circle),
// and every change rolls the number up. Tapping it goes back to the newest messages and resumes live. The
// pair rides above the composer's suggestion strip while that strip covers the
// bottom of the chat, and settles back when it goes.
import React, { useEffect, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { CaretDown } from 'phosphor-react';
import { usePausedNewCount } from '../../hooks/usePausedNewCount';
import { RESUME_COUNT_STEP_MS, resumeCountText, resumeLabel } from './resumeLabel';

const EASE = [0.2, 0.8, 0.2, 1] as const;
const POP = { duration: 0.18, ease: EASE };
const ROLL = { duration: 0.2, ease: EASE };
const LIFT = { duration: 0.2, ease: EASE };
const MARK = 'absolute inset-0 flex items-center justify-center';

/** The count the circle shows: follows `count`, but steps at most every
 *  `RESUME_COUNT_STEP_MS`, so in fast chat each roll lands before the next. */
function useSteppedCount(count: number): number {
  const [shown, setShown] = useState(count);
  const lastStep = useRef(0);
  useEffect(() => {
    if (count === shown) return;
    // Back to nothing (a resume) goes at once; a rise waits out the step.
    const wait = count === 0 ? 0 : Math.max(0, RESUME_COUNT_STEP_MS - (performance.now() - lastStep.current));
    const timer = setTimeout(() => {
      lastStep.current = performance.now();
      setShown(count);
    }, wait);
    return () => clearTimeout(timer);
  }, [count, shown]);
  return shown;
}

const ResumeButton: React.FC<{ sliceKey: string; onResume: () => void }> = ({ sliceKey, onResume }) => {
  // Its own leaf: the count re-renders on every flush while paused, and only
  // this button should.
  const count = usePausedNewCount(sliceKey, true);
  const shown = useSteppedCount(count);
  const text = resumeCountText(shown);
  const counting = text !== '';
  return (
    <motion.button
      type="button"
      onClick={onResume}
      aria-label={resumeLabel(count)}
      initial={{ opacity: 0, scale: 0.8 }}
      animate={{ opacity: 1, scale: 1 }}
      exit={{ opacity: 0, scale: 0.8 }}
      whileTap={{ scale: 0.92 }}
      transition={POP}
      // The glaze of the composer's round button below it, frosted like the
      // glaze controls that float over busy content, so the count reads over
      // chat text.
      className="pointer-events-auto absolute right-2.5 top-0 h-10 w-10 chrome-glaze chrome-glaze--control chrome-glaze--frosted text-textPrimary"
    >
      <motion.span
        aria-hidden
        className={MARK}
        initial={false}
        animate={{ opacity: counting ? 0 : 1, scale: counting ? 0.7 : 1 }}
        transition={POP}
      >
        <CaretDown size={18} weight="bold" />
      </motion.span>
      {/* Every change rolls: the old number rises out as the new one rises in,
          clipped to the number's own line, so two are never stacked. */}
      <span aria-hidden className={MARK}>
        <span className="relative inline-flex h-4 items-center justify-center overflow-hidden">
          <AnimatePresence initial={false} mode="popLayout">
            {counting && (
              <motion.span
                key={text}
                className="block text-[13px] font-bold leading-4 tabular-nums"
                initial={{ opacity: 0, y: '100%' }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: '-100%' }}
                transition={ROLL}
              >
                {text}
              </motion.span>
            )}
          </AnimatePresence>
        </span>
      </span>
    </motion.button>
  );
};

export const PausedOverlay: React.FC<{
  paused: boolean;
  /** The chat's slice key for the count; empty counts nothing (a replay). */
  sliceKey: string;
  /** Show the reply hint (paused, the room can take a reply, not learned yet). */
  hint: boolean;
  /** The hint's words, fitted to its column ("Swipe left on a message to reply", or shorter). */
  hintWords: string;
  /** How far to rise over the composer's strip. */
  lift: number;
  onResume: () => void;
}> = ({ paused, sliceKey, hint, hintWords, lift, onResume }) => (
  <motion.div
    className="pointer-events-none absolute bottom-2 left-0 right-0 z-10 grid h-10 items-center"
    // The hint in the middle column: centred while both sides have room, and
    // in a narrow chat it gives up the centre (never the button's 56 px) before
    // it would cut its words.
    style={{ gridTemplateColumns: 'minmax(10px, 1fr) minmax(0, max-content) minmax(56px, 1fr)' }}
    initial={false}
    animate={{ y: -lift }}
    transition={LIFT}
  >
    <AnimatePresence>
      {paused && hint && (
        <motion.span
          key="hint"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={POP}
          className="col-start-2 glass-badge truncate rounded-full px-2 py-0.5 text-[10.5px] text-textMuted"
        >
          {hintWords}
        </motion.span>
      )}
    </AnimatePresence>
    <AnimatePresence>
      {paused && <ResumeButton key="resume" sliceKey={sliceKey} onResume={onResume} />}
    </AnimatePresence>
  </motion.div>
);
