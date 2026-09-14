// A full-screen layer that pushes in from the right and slides back out.
//
// The drill-in screens (settings panels, cosmetics, a category's streams) used
// to appear and vanish instantly, which reads as a cut rather than navigation:
// nothing tells you whether you went deeper or sideways, and coming back feels
// like the app blinked. Sliding in from the trailing edge is the standard
// push-navigation grammar on both platforms, and it makes back unmistakably
// "out" because the layer leaves the way it arrived.
//
// Shared rather than three copies of the same framer config, so the screens
// cannot drift apart, and so the timing is tuned in one place.
import React from 'react';
import { AnimatePresence, motion } from 'framer-motion';

// Matches the tab-bar slide in MobileApp, just slightly longer because this
// layer travels the full width rather than 28px.
const EASE = [0.2, 0.8, 0.2, 1] as const;
const DURATION = 0.22;

interface Props {
  open: boolean;
  /** Distinct per screen, so two drill-ins never share a presence slot. */
  layerKey: string;
  className?: string;
  style?: React.CSSProperties;
  children: React.ReactNode;
}

export const DrillInScreen: React.FC<Props> = ({
  open,
  layerKey,
  className = '',
  style,
  children,
}) => (
  // AnimatePresence has to sit OUTSIDE the conditional: it is what keeps the
  // subtree mounted long enough to play the exit. A screen that returns null
  // when closed can never animate away.
  <AnimatePresence>
    {open && (
      <motion.div
        key={layerKey}
        className={className}
        style={style}
        initial={{ x: '100%' }}
        animate={{ x: 0 }}
        exit={{ x: '100%' }}
        transition={{ duration: DURATION, ease: EASE }}
      >
        {children}
      </motion.div>
    )}
  </AnimatePresence>
);
