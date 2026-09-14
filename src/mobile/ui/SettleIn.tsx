// Cards settling into place the first time a list appears, so arriving on a
// screen lands on something with a bit of life rather than a grid that snaps in
// fully formed.
//
// TWO THINGS HERE ARE LOAD-BEARING, and both were learned the hard way.
//
// It does NOT use framer's `initial`. Every mobile screen renders inside the
// tab switcher's `AnimatePresence initial={false}` (MobileApp), which exists so
// the first tab does not slide in on boot. That flag rides PresenceContext and
// cascades to every motion component beneath it, so an `initial` anywhere down
// here is suppressed on the subtree's first render - exactly the launch and
// post-setup case this animation is for. Animating between two `animate` values
// instead sidesteps the context entirely.
//
// It latches. Pulling to refresh, appending the next page of an infinite
// scroll, or an item arriving on its own must not replay the whole shuffle.
// Once settled, later items simply appear.
//
// The line between "same list" and "new list" is the `resetKey`, and it is a
// judgement call per screen rather than a rule. Refreshing keeps the key,
// because the same cards come back the same shape. Switching between the card
// and list layouts changes it, because every row is reshaped and resized and
// the swap deserves to be animated rather than snapped. Running a new search
// changes it too.
//
// It needs no reduced-motion special case: framer snaps a reduced-motion
// animation to its target, and the target here is the item visible in place.
import React, { useEffect, useState } from 'react';
import { motion } from 'framer-motion';

// Per-item step, matching the stagger already used by the settings panels, and
// a cap so a long list does not leave the last card drifting in a second after
// the first.
const STEP = 0.045;
const MAX_DELAY = 0.4;

/**
 * Returns whether the settle has been released.
 *
 * `ready` is whether the list is actually showing its items. Skeletons and
 * empty states pass false, or the animation is spent before there is anything
 * to animate.
 *
 * `resetKey` identifies WHICH list is on screen, and changing it replays the
 * settle. That is the difference between a list growing and a list being
 * replaced: appending the next page of an infinite scroll keeps the same key so
 * new rows just appear, while running a new search swaps the key and the fresh
 * results settle in. Leave it out for a list that only ever has one identity.
 */
export function useSettleIn(ready: boolean, resetKey: string | number = ''): boolean {
  // Stamped with the list it belongs to and compared during render, rather than
  // cleared from an effect when the key changes. Writing state from an effect
  // is an error under this repo's hook rules, and it would also paint one frame
  // of the previous list's settled state over the new one.
  const [state, setState] = useState<{ key: string | number; done: boolean }>({
    key: resetKey,
    done: false,
  });
  const settled = state.key === resetKey && state.done;

  useEffect(() => {
    if (!ready || settled) return;
    // One frame at the start state, then release. Without the frame both values
    // land in the same commit and there is nothing to animate between.
    const id = requestAnimationFrame(() => setState({ key: resetKey, done: true }));
    return () => cancelAnimationFrame(id);
  }, [ready, resetKey, settled]);

  return settled;
}

/** Wraps one item in a list. `index` drives the stagger. */
export const SettleIn: React.FC<{
  index: number;
  settled: boolean;
  className?: string;
  children: React.ReactNode;
}> = ({ index, settled, className, children }) => {
  // Steps down to a plain div once the entrance has finished playing. There is
  // nothing left to animate at that point, and a motion component that stays
  // mounted keeps a live subscription and its own compositor layer. That is
  // invisible on a short list and very much not on a deep scroll through a busy
  // category, where several hundred of these sit mounted for the rest of the
  // session because the grid stays behind the player.
  const [done, setDone] = useState(false);
  // Adjust-state-during-render rather than an effect: the repo's lint rules flag
  // setState inside effects, and this is the pattern React documents for
  // resetting state when a prop changes.
  const [wasSettled, setWasSettled] = useState(settled);
  if (wasSettled !== settled) {
    setWasSettled(settled);
    if (!settled) setDone(false);
  }

  if (settled && done) return <div className={className}>{children}</div>;

  return (
    <motion.div
      className={className}
      // Explicit: framer then starts at whatever `animate` currently says, which
      // on the first commit is the start state.
      initial={false}
      animate={settled ? { opacity: 1, y: 0, scale: 1 } : { opacity: 0, y: 14, scale: 0.97 }}
      transition={{
        duration: 0.34,
        ease: [0.16, 1, 0.3, 1],
        delay: settled ? Math.min(index * STEP, MAX_DELAY) : 0,
      }}
      onAnimationComplete={() => setDone(true)}
    >
      {children}
    </motion.div>
  );
};
