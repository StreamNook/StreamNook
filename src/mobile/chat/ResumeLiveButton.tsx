// What floats over a paused phone chat: the round resume button at the bottom
// right, and the swipe-to-reply hint until it has been learned.
//
// The button is a chevron in an accent circle; once new messages arrive, the
// chevron turns into how many (capped "99+", so the number always fits the
// circle). Tapping it goes back to the newest messages and resumes live. The
// pair rides above the composer's suggestion strip while that strip covers the
// bottom of the chat, and settles back when it goes.
import React from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { CaretDown } from 'phosphor-react';
import { usePausedNewCount } from '../../hooks/usePausedNewCount';
import { fitCount } from '../../utils/fitNumber';
import { resumeLabel } from './resumeLabel';

const EASE = [0.2, 0.8, 0.2, 1] as const;
const POP = { duration: 0.18, ease: EASE };
const LIFT = { duration: 0.2, ease: EASE };
const MARK = 'absolute inset-0 flex items-center justify-center';

const ResumeButton: React.FC<{ sliceKey: string; onResume: () => void }> = ({ sliceKey, onResume }) => {
  // Its own leaf: the count re-renders on every flush while paused, and only
  // this button should.
  const count = usePausedNewCount(sliceKey, true);
  const counting = count > 0;
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
      className="pointer-events-auto absolute right-2.5 top-0 h-10 w-10 rounded-full bg-accent sn-send-ink ring-1 ring-white/15 shadow-[0_4px_14px_rgba(0,0,0,0.35)]"
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
      <motion.span
        aria-hidden
        className={`${MARK} text-[13px] font-bold leading-none tabular-nums`}
        initial={false}
        animate={{ opacity: counting ? 1 : 0, scale: counting ? 1 : 0.7 }}
        transition={POP}
      >
        {counting ? fitCount(count) : ''}
      </motion.span>
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
