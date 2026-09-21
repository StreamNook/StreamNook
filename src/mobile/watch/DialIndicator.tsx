// The readout for the player's swipe dials: brightness on the left half of
// the picture, volume on the right. One capsule in the middle of the video,
// shown only while a finger is dialling and for a beat after it lifts, so the
// final value can be read. It never takes a touch: the band underneath owns
// the gesture.
import React from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { SpeakerHigh, SpeakerLow, SpeakerNone, Sun, SunDim } from 'phosphor-react';

export type DialAxis = 'brightness' | 'volume';

export interface DialReading {
  axis: DialAxis;
  /** 0..1 */
  value: number;
}

const iconFor = (axis: DialAxis, value: number) => {
  if (axis === 'brightness') return value < 0.5 ? SunDim : Sun;
  if (value <= 0.001) return SpeakerNone;
  return value < 0.5 ? SpeakerLow : SpeakerHigh;
};

export const DialIndicator: React.FC<{ dial: DialReading | null }> = ({ dial }) => (
  <AnimatePresence>
    {dial && (
      <motion.div
        key={dial.axis}
        className="absolute inset-0 z-30 flex items-center justify-center pointer-events-none"
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        transition={{ duration: 0.16 }}
      >
        <motion.div
          className="chrome-glaze chrome-glaze--frosted flex items-center gap-3 px-4 h-11 text-white"
          initial={{ scale: 0.94 }}
          animate={{ scale: 1 }}
          exit={{ scale: 0.96 }}
          transition={{ type: 'spring', stiffness: 420, damping: 32 }}
          role="status"
          aria-live="polite"
          aria-label={`${dial.axis === 'brightness' ? 'Brightness' : 'Volume'} ${Math.round(dial.value * 100)}%`}
        >
          {React.createElement(iconFor(dial.axis, dial.value), { size: 20, weight: 'fill' })}
          {/* A short track rather than a number alone: the eye reads position
              faster than it reads digits while the hand is still moving. */}
          <div className="relative w-28 h-[3px] rounded-full bg-white/25 overflow-hidden">
            <div
              className="absolute inset-y-0 left-0 rounded-full bg-accent"
              style={{ width: `${Math.round(dial.value * 100)}%` }}
            />
          </div>
          <span className="text-[13px] font-semibold tabular-nums w-9 text-right">
            {Math.round(dial.value * 100)}%
          </span>
        </motion.div>
      </motion.div>
    )}
  </AnimatePresence>
);
