// The teaching placeholder's words over an empty message box: each hint
// cross-fades in with a small rise and out with a small rise, transform and
// opacity only. The field under it keeps its own placeholder (transparent via
// `textarea[data-hint-layer]`), so screen readers still hear the prompt.
// Shown only while the hints cycle (`useComposerHint`), never under reduced
// motion, so the transform here always runs at full motion.
import { AnimatePresence, motion } from 'framer-motion';
import type { CSSProperties } from 'react';

const EASE = [0.2, 0.8, 0.2, 1] as const;

export function ComposerHintLayer({
  hintKey,
  wide,
  narrow,
  className = '',
  style,
}: {
  /** Changes with each hint: the swap animates on it. */
  hintKey: string;
  wide: string;
  /** The words for a narrow composer row (`.sn-composer-row[data-narrow]`);
   *  absent where the caller already picked words that fit. */
  narrow?: string;
  /** The field's text box: font, padding and placeholder ink. */
  className?: string;
  style?: CSSProperties;
}) {
  return (
    <div aria-hidden className={`pointer-events-none absolute inset-0 z-[1] overflow-hidden ${className}`} style={style}>
      <div className="relative h-full">
        <AnimatePresence initial={false}>
          <motion.span
            key={hintKey}
            className="absolute inset-x-0 top-0 truncate whitespace-nowrap"
            initial={{ opacity: 0, y: 6 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -6 }}
            transition={{ duration: 0.3, ease: EASE }}
          >
            {narrow === undefined ? (
              wide
            ) : (
              <>
                <span className="sn-hint-wide">{wide}</span>
                <span className="sn-hint-narrow">{narrow}</span>
              </>
            )}
          </motion.span>
        </AnimatePresence>
      </div>
    </div>
  );
}
